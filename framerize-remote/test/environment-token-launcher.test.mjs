import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { createServer, get } from 'node:http';
import { connect } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import test from 'node:test';

const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const token = 'synthetic-loopback-token';

async function bounded(promise, milliseconds, label) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolveUnused, reject) => {
      timer = setTimeout(() => reject(new Error(label)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function refused(port) {
  await assert.rejects(new Promise((resolveReady, reject) => {
    const socket = connect({ host: '127.0.0.1', port });
    socket.once('error', reject);
    socket.once('connect', () => { socket.destroy(); resolveReady(); });
    socket.setTimeout(1000, () => socket.destroy(new Error('port refusal timed out')));
  }), { code: 'ECONNREFUSED' });
}

async function request(port, path, headers = {}) {
  return new Promise((resolveResult, reject) => {
    const outgoing = get({ host: '127.0.0.1', port, path, headers, agent: false }, response => {
      response.resume();
      response.once('end', () => resolveResult(response.statusCode));
      response.once('error', reject);
    });
    outgoing.once('error', reject);
    outgoing.setTimeout(1500, () => outgoing.destroy(new Error('HTTP request timed out')));
  });
}

async function readReady(child, closed) {
  let output = '';
  const ready = new Promise(resolveReady => {
    child.stdout.on('data', chunk => {
      output += chunk;
      for (const line of output.split('\n').slice(0, -1)) {
        const message = JSON.parse(line);
        if (message.ready === true) resolveReady(message);
      }
    });
  });
  return bounded(Promise.race([ready, closed.then(() => {
    throw new Error('launcher closed before readiness');
  })]), 3000, 'launcher readiness timed out');
}

test('real launcher retains a mixed-case local token before serving', {
  timeout: 10000, skip: process.platform === 'win32' ? 'POSIX child signal lifecycle' : false,
}, async context => {
  const directory = await mkdtemp(join(tmpdir(), 'remote-env-launcher-'));
  let child, closed, remotePort, localPort, cleanupPromise;
  const sockets = new Set();
  const requests = [];
  const remote = createServer((incoming, response) => {
    requests.push({ path: incoming.url, authorization: incoming.headers.authorization });
    response.setHeader('content-type', 'application/json');
    response.end(JSON.stringify({ status: 'ready', apiVersion: 1,
      capabilities: { responses: true, chatCompletions: true, streaming: true } }));
  });
  remote.on('connection', socket => { sockets.add(socket); socket.once('close', () => sockets.delete(socket)); });
  const cleanup = () => cleanupPromise ??= (async () => {
    try {
      if (child && child.exitCode === null && child.signalCode === null) {
        child.kill('SIGTERM');
        await bounded(closed, 1000, 'graceful cleanup expired').catch(async () => {
          child.kill('SIGKILL'); await bounded(closed, 1500, 'forced cleanup expired');
        });
      }
    } finally {
      for (const socket of sockets) socket.destroy();
      await new Promise(resolveClosed => remote.close(resolveClosed));
      try { if (localPort) await refused(localPort); if (remotePort) await refused(remotePort); }
      finally { await rm(directory, { recursive: true, force: true }); }
    }
  })();
  context.after(cleanup);
  remote.listen(0, '127.0.0.1');
  await bounded(once(remote, 'listening'), 2000, 'fixture startup timed out');
  remotePort = remote.address().port;
  const profilePath = join(directory, 'framerize-remote.json');
  const profile = `${JSON.stringify({ remote: `http://127.0.0.1:${remotePort}`, host: '127.0.0.1', port: 0 })}\n`;
  await writeFile(profilePath, profile, { mode: 0o600 });
  const guardPath = join(directory, 'forbidden-commands');
  const preloadPath = join(directory, 'environment.mjs');
  await writeFile(preloadPath, [
    "import commands from 'node:child_process';",
    "import { appendFileSync } from 'node:fs';",
    "import { syncBuiltinESMExports } from 'node:module';",
    "for (const name of ['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync', 'fork']) {",
    `  commands[name] = () => { appendFileSync(${JSON.stringify(guardPath)}, name + '\\n'); throw new Error('External command forbidden'); };`,
    '}', 'syncBuiltinESMExports();',
    'process.env = new Proxy(process.env, { get(target, property) {',
    "  if (typeof property !== 'string') return Reflect.get(target, property);",
    '  const key = Object.keys(target).find(name => name.toLowerCase() === property.toLowerCase());',
    '  return target[key ?? property];',
    '} });',
  ].join('\n'), { mode: 0o600 });
  child = spawn(process.execPath, [join(packageRoot, 'bin/frm.js'), 'serve', '--json'], {
    cwd: directory, stdio: ['ignore', 'pipe', 'pipe'], env: {
      PATH: dirname(process.execPath), HOME: directory, USERPROFILE: directory, FRM_HOME: directory,
      NODE_OPTIONS: `--import=${pathToFileURL(preloadPath).href}`, NODE_DISABLE_COMPILE_CACHE: '1',
      FrM_Local_Token: token,
    },
  });
  closed = new Promise((resolveClosed, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => resolveClosed({ code, signal }));
  });
  let stderr = '';
  child.stderr.on('data', chunk => { stderr += chunk; });
  const ready = await readReady(child, closed);
  localPort = ready.listening.port;
  const health = await request(localPort, '/health');
  const unauthenticated = await request(localPort, '/ready');
  const authenticated = await request(localPort, '/ready', { authorization: `Bearer ${token}` });
  child.kill('SIGTERM');
  const exit = await bounded(closed, 2000, 'launcher shutdown timed out');
  assert.equal(await readFile(profilePath, 'utf8'), profile);
  await assert.rejects(stat(guardPath), { code: 'ENOENT' });
  await cleanup();
  context.diagnostic(JSON.stringify({ runtime: process.version, packageRoot, health, unauthenticated,
    authenticated, exit, remoteRequests: requests.length, portsRefused: true, syntheticEnvironment: true }));
  assert.deepEqual(exit, { code: 0, signal: null });
  assert.equal(stderr, '');
  assert.equal(health, 200);
  assert.equal(authenticated, 200);
  assert.ok(requests.every(entry => entry.path === '/ready' && entry.authorization === undefined));
  assert.equal(unauthenticated, 401, 'the configured local token must protect readiness');
});
