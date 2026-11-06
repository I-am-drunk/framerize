import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import { createServer, get } from 'node:http';
import { isIP } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const manifest = JSON.parse(await readFile(join(packageRoot, 'package.json'), 'utf8'));
assert.equal(typeof manifest.bin?.frm, 'string');
const launcher = resolve(packageRoot, manifest.bin.frm);

async function state(context, setting) {
  const root = await mkdtemp(join(tmpdir(), `frm-${setting}-precedence-`));
  context.after(() => rm(root, { recursive: true, force: true }));
  const env = { HOME: root, USERPROFILE: root, TMPDIR: root, TMP: root, TEMP: root,
    PATH: '', FRM_HOME: join(root, 'profile'), [`FRM_${setting.toUpperCase()}`]: 'invalid value',
    FRM_REMOTE: 'http://127.0.0.1:43123', NODE_DISABLE_COMPILE_CACHE: '1' };
  return { root, env };
}

function completed(root, env, args) {
  return new Promise(resolve => execFile(process.execPath, [launcher, ...args],
    { cwd: root, env, timeout: 5000, killSignal: 'SIGKILL', maxBuffer: 65536 },
    (error, stdout, stderr) => resolve({ code: error?.code ?? 0, stdout, stderr })));
}

for (const setting of ['port', 'host']) {
  test(`setup --${setting} replaces an invalid FRM_${setting.toUpperCase()} before validation`,
    { timeout: 8000 }, async context => {
      const { root, env } = await state(context, setting);
      const value = setting === 'port' ? '0' : '127.0.0.1';
      const result = await completed(root, env, ['setup', `--${setting}`, value, '--json']);
      assert.equal(result.code, 0, result.stderr);
      const profile = JSON.parse(await readFile(join(env.FRM_HOME, 'framerize-remote.json'), 'utf8'));
      assert.equal(profile[setting], setting === 'port' ? 0 : value);
      assert.equal(profile.remote, env.FRM_REMOTE);
    });

  for (const command of ['setup', 'serve']) {
    test(`${command}: invalid FRM_${setting.toUpperCase()} fails without an override and writes no profile`,
      { timeout: 8000 }, async context => {
        const { root, env } = await state(context, setting);
        const result = await completed(root, env, [command, '--json']);
        assert.equal(result.code, 1);
        assert.match(result.stderr + result.stdout, /listen_invalid/);
        assert.deepEqual(await readdir(root), []);
      });
  }
}

function health(port, host = '127.0.0.1') {
  return new Promise((resolve, reject) => {
    const request = get({ host, port, path: '/health', agent: false,
      signal: AbortSignal.timeout(2000) }, response => {
      const address = response.socket.remoteAddress;
      response.once('error', reject);
      response.once('end', () => resolve({ status: response.statusCode, address }));
      response.resume();
    });
    request.once('error', reject);
  });
}

async function bounded(promise) {
  let timer;
  try { return await Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error('launcher did not settle')), 5000);
  })]); } finally { clearTimeout(timer); }
}

for (const command of ['serve', 'setup']) {
 for (const setting of ['port', 'host']) {
  test(`${command} --${setting} replaces an invalid FRM_${setting.toUpperCase()} and closes its real listener`,
  { timeout: 15000 }, async context => {
    const { root, env } = await state(context, setting);
    const calls = [];
    const remote = createServer((request, response) => {
      calls.push(request.url);
      request.resume();
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ apiVersion: 1, status: 'ready', capabilities:
        { responses: true, chatCompletions: true, streaming: true } }));
    });
    context.after(async () => {
      remote.closeAllConnections();
      if (remote.listening) await new Promise(resolve => remote.close(resolve));
    });
    const remoteListening = once(remote, 'listening');
    remote.listen(0, '127.0.0.1');
    await bounded(remoteListening);
    env.FRM_REMOTE = `http://127.0.0.1:${remote.address().port}`;
    const host = setting === 'host' ? 'localhost' : '127.0.0.1';
    const child = spawn(process.execPath, [launcher, command, '--port', '0', '--json',
      ...(command === 'setup' ? ['--start'] : []),
      ...(setting === 'host' ? ['--host', host] : [])],
      { cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'] });
    const closed = once(child, 'close');
    closed.catch(() => {});
    let errors = '';
    child.stderr.setEncoding('utf8').on('data', text => { errors += text; });
    context.after(async () => {
      if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
      await bounded(closed);
    });
    const lines = createInterface({ input: child.stdout });
    context.after(() => lines.close());
    const ready = (async () => {
      for await (const line of lines) {
        const event = JSON.parse(line);
        if (event.listening) return event.listening;
      }
      throw new Error(`frm exited before listening: ${errors}`);
    })();
    const listening = await bounded(ready);
    assert.equal(listening.host, host);
    const { port } = listening;
    const peer = await health(port, host);
    assert.equal(peer.status, 200);
    assert.notEqual(isIP(peer.address), 0);
    context.diagnostic(JSON.stringify({ requestedHost: host, peer: peer.address, port }));
    assert.deepEqual(calls, ['/ready']);
    child.kill('SIGTERM');
    assert.deepEqual(await bounded(closed), process.platform === 'win32' ? [null, 'SIGTERM'] : [0, null]);
    // localhost can select another address family after this listener closes.
    await assert.rejects(health(port, peer.address), { code: 'ECONNREFUSED' });
    const entries = await readdir(root, { recursive: true, withFileTypes: true });
    assert.deepEqual(entries.filter(entry => !entry.isDirectory()).map(entry => entry.name),
      command === 'setup' ? ['framerize-remote.json'] : []);
    if (command === 'setup') {
      const saved = JSON.parse(await readFile(join(env.FRM_HOME, 'framerize-remote.json'), 'utf8'));
      assert.equal(saved.host ?? '127.0.0.1', host);
      assert.equal(saved.port, 0);
      assert.equal(saved.remote, env.FRM_REMOTE);
    }
  });
 }
}
