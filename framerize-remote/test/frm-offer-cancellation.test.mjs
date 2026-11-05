import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { once, getEventListeners } from 'node:events';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { createServer, get } from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/clientCli.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { offerClientSetup } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function bounded(promise, message, milliseconds = 3000) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

const abortError = () => Object.assign(new Error('Synthetic prompt cancelled'), { name: 'AbortError' });

for (const stage of ['intro', 'install', 'update']) test(`abort releases the pending ${stage} offer`, async () => {
  const controller = new AbortController(), reached = Promise.withResolvers();
  const questions = [], commands = [];
  let release, output = '';
  const hold = (question, signal) => new Promise((resolve, reject) => {
    questions.push(question);
    const abort = () => { signal?.removeEventListener('abort', abort); reject(abortError()); };
    release = () => { signal?.removeEventListener('abort', abort); resolve('no'); };
    signal?.addEventListener('abort', abort, { once: true });
    reached.resolve();
  });
  const io = { stdout(text) { output += text; }, stderr() {},
    ask: stage === 'intro' ? hold : async () => 'yes' };
  const native = '/synthetic/home/.local/bin/claude';
  const deps = { platform: 'linux', env: { PATH: '/synthetic/home/.local/bin' }, home: '/synthetic/home',
    stateDir: '/synthetic/state', target: { origin: 'http://127.0.0.1:8788' }, io, prompt: hold,
    probe: path => stage === 'update' && path.replaceAll('\\', '/') === native,
    realPath: path => path, read: async () => null, fetch: async () => '2.1.301', runner: {
      async run(command) { commands.push(command); return 0; },
      async capture() { return '2.1.300 (Claude Code)'; },
    } };
  const pending = offerClientSetup('/missing/profile.json', io, {}, deps, controller.signal);
  let settled;
  try {
    await bounded(reached.promise, 'Offer did not reach its prompt');
    controller.abort();
    settled = await bounded(pending.then(() => true), 'Offer ignored abort', 250).catch(() => false);
  } finally { release?.(); await pending.catch(() => {}); }
  assert.equal(settled, true, 'The offer must finish without an answer');
  assert.equal(questions.length, 1);
  if (stage === 'update') assert.match(questions[0], /Update Claude Code/);
  assert.deepEqual(commands, []);
  assert.doesNotMatch(output, /Start a client|Install Codex/);
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});

async function health(port) {
  return new Promise((resolve, reject) => {
    const request = get(`http://127.0.0.1:${port}/health`, response => {
      response.resume();
      response.once('end', () => resolve(response.statusCode));
    });
    request.setTimeout(1000, () => request.destroy(new Error('Health request timed out')));
    request.once('error', reject);
  });
}

async function unusedPort() {
  const server = createServer();
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const port = server.address().port;
  await new Promise(resolve => server.close(resolve));
  return port;
}

const cancellations = ['SIGINT', 'SIGTERM', 'Ctrl+C'];
for (const mode of ['up', 'setup --start', 'bare frm'])
for (const signal of mode === 'bare frm' ? [...cancellations, 'decline'] : cancellations) {
  const name = signal === 'decline' ? 'bare frm starts its listener after declining the offer'
    : `${mode} cancels its optional prompt with ${signal}`;
  test(name, {
    timeout: 10000, skip: process.platform === 'win32'
      ? 'POSIX signals and terminal input are exercised only on POSIX' : false,
  }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'frm-offer-cancellation-'));
    const profile = join(root, 'framerize-remote.json');
    const guard = join(root, 'terminal.cjs');
    const commands = join(root, 'commands.jsonl');
    const sockets = new Set(), requests = [];
    let child, closed, stdout = '', stderr = '', answered = false;
    const remote = createServer((request, response) => {
      requests.push(request.url);
      request.resume();
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ apiVersion: 1, status: 'ready',
        capabilities: { responses: true, chatCompletions: true, streaming: true } }));
    });
    remote.on('connection', socket => {
      sockets.add(socket);
      socket.once('close', () => sockets.delete(socket));
    });
    context.after(async () => {
      try {
        if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
        if (closed) await bounded(closed, 'Launcher cleanup timed out');
      } finally {
        for (const socket of sockets) socket.destroy();
        if (remote.listening) await new Promise(resolve => remote.close(resolve));
        await rm(root, { recursive: true, force: true });
      }
    });
    await writeFile(guard, `
const { appendFileSync } = require('node:fs');
const processes = require('node:child_process');
Object.defineProperty(process.stdin, 'isTTY', { value: true });
Object.defineProperty(process.stdout, 'isTTY', { value: true });
// A synthetic unsupported platform prevents service-manager activity in frm up.
Object.defineProperty(process, 'platform', { value: 'freebsd' });
for (const method of ['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync', 'fork']) {
  processes[method] = () => {
    appendFileSync(${JSON.stringify(commands)}, method + '\\n');
    throw new Error('External commands are forbidden in this fixture');
  };
}
require('node:module').syncBuiltinESMExports();
`, { mode: 0o600 });
    await new Promise(resolve => remote.listen(0, '127.0.0.1', resolve));
    const remotePort = remote.address().port;
    const origin = `http://127.0.0.1:${remotePort}`;
    const localPort = mode === 'bare frm' ? await unusedPort() : 0;
    const args = mode === 'bare frm' ? [] : mode === 'up'
      ? ['up', origin] : ['setup', '--remote', origin, '--start'];
    const reached = Promise.withResolvers(), started = Promise.withResolvers();
    child = spawn(process.execPath, ['--require', guard, join(packageRoot, 'bin/frm.js'), ...args], {
      cwd: root, stdio: ['pipe', 'pipe', 'pipe'],
      env: { HOME: root, USERPROFILE: root, FRM_HOME: root, FRM_PORT: String(localPort), PATH: '',
        TMPDIR: root, TMP: root, TEMP: root, NODE_DISABLE_COMPILE_CACHE: '1' },
    });
    closed = new Promise((resolve, reject) => {
      child.once('error', reject);
      child.once('close', (code, signal) => resolve({ code, signal }));
    });
    child.stdout.setEncoding('utf8').on('data', chunk => {
      stdout += chunk;
      if (mode === 'bare frm' && !answered && stdout.includes('Pairing link or server address: ')) {
        answered = true;
        child.stdin.write(`${origin}\n`);
      }
      if (stdout.includes('Set up Claude Code or Codex')) reached.resolve();
      if (stdout.includes('Local API:')) started.resolve();
    });
    child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk; });
    await bounded(Promise.race([reached.promise, closed.then(exit => {
      throw new Error(`Launcher exited before the offer: ${JSON.stringify(exit)} ${stderr}`);
    })]), 'Launcher did not reach the offer');
    const saved = await readFile(profile);
    const port = Number(/Local API:?\s+http:\/\/127\.0\.0\.1:(\d+)\/v1/.exec(stdout)?.[1]);
    if (mode === 'bare frm') {
      assert.equal(answered, true, 'Bare frm must first ask for its remote');
      assert.equal(port, localPort);
    }
    if (mode === 'setup --start') assert.equal(await health(port), 200);
    if (signal === 'decline') {
      child.stdin.write('no\n');
      await bounded(Promise.race([started.promise, closed.then(exit => {
        throw new Error(`Launcher exited before serving: ${JSON.stringify(exit)} ${stderr}`);
      })]), 'Bare frm did not start after declining the offer');
      assert.equal(await health(port), 200);
      assert.equal(child.kill('SIGTERM'), true);
    } else if (signal === 'Ctrl+C') child.stdin.write('\x03');
    else assert.equal(child.kill(signal), true);
    const exit = await bounded(closed, 'Offer ignored cancellation', 1500).catch(() => null);
    const listenerStillOpen = mode !== 'up'
      ? await health(port).then(() => true, () => false) : false;
    if (!exit) child.kill('SIGKILL');
    await bounded(closed, 'Launcher cleanup timed out');
    const after = await readFile(profile);
    if (mode !== 'up') await assert.rejects(health(port), { code: 'ECONNREFUSED' });
    assert.deepEqual(after, saved, 'Cancelling an optional offer must preserve the saved profile');
    assert.deepEqual((await readdir(root)).sort(), ['framerize-remote.json', 'terminal.cjs']);
    for (const socket of sockets) socket.destroy();
    await new Promise(resolve => remote.close(resolve));
    await assert.rejects(health(remotePort), { code: 'ECONNREFUSED' });
    const observed = { exit, listenerStillOpen, errorOutput: stderr,
      stopped: /Stopped\./.test(stdout), offeredInstall: /Install (Claude|Codex)/.test(stdout) };
    context.diagnostic(JSON.stringify({ mode, signal, ...observed }));
    assert.deepEqual(requests, ['/ready']);
    assert.deepEqual(observed, { exit: { code: 0, signal: null }, listenerStillOpen: false,
      errorOutput: '', stopped: mode === 'setup --start' || signal === 'decline', offeredInstall: false });
  });
}
