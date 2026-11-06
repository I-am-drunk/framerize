import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { createConnection } from 'node:net';
import { access, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const oldToken = 'synthetic-old-origin-token';
const explicitToken = 'synthetic-explicit-destination-token';
const localToken = 'synthetic-local-only-token';
const classify = value => value === undefined ? 'none' : value === `Bearer ${oldToken}`
  ? 'old' : value === `Bearer ${explicitToken}` ? 'explicit'
    : value === `Bearer ${localToken}` ? 'local' : 'other';

async function bounded(pending) {
  let timer;
  try { return await Promise.race([pending, new Promise((resolveUnused, reject) => {
    timer = setTimeout(() => reject(new Error('Owned operation exceeded three seconds')), 3000);
  })]); } finally { clearTimeout(timer); }
}

function connectResult(port) {
  return new Promise(resolveResult => {
    const socket = createConnection({ host: '127.0.0.1', port });
    const finish = result => {
      socket.once('close', () => resolveResult(result));
      socket.destroy();
    };
    socket.once('connect', () => finish('connected'));
    socket.once('error', error => finish(error.code));
    socket.setTimeout(1000, () => finish('timeout'));
  });
}

const cases = [
  { name: 'same origin retains its saved credential', same: true, expected: 'old' },
  { name: 'changed plain origin does not inherit the previous credential', expected: 'none' },
  { name: 'changed origin uses an explicitly supplied credential', explicit: true, expected: 'explicit' },
  { name: 'changed tokenless pair link omits the previous credential', pair: true, expected: 'none' },
];

for (const sample of cases) test(sample.name, { timeout: 8000 }, async context => {
  const root = await mkdtemp(join(tmpdir(), 'frm-origin-token-fixture-'));
  const listeners = [], sockets = new Set(), seen = [];
  let child, closed, stdout = '', stderr = '', cleaned = false;
  const cleanup = async () => {
    if (cleaned) return;
    try {
      if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
      if (closed) await bounded(closed);
    } finally {
      const released = [...sockets].map(socket => new Promise(resolveClose => socket.once('close', resolveClose)));
      for (const socket of sockets) socket.destroy();
      await bounded(Promise.all(released));
      await Promise.all(listeners.map(server => new Promise(resolveClose => server.close(resolveClose))));
      await rm(root, { recursive: true, force: true });
      cleaned = true;
    }
  };
  context.after(cleanup);
  const listen = async side => {
    const server = createServer((request, response) => {
      seen.push({ side, path: request.url, authorization: classify(request.headers.authorization) });
      response.writeHead(503, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ apiVersion: 1, status: 'unavailable', reason: 'upstream_unavailable',
        capabilities: { responses: true, chatCompletions: true, streaming: true } }));
    });
    listeners.push(server);
    server.on('connection', socket => {
      sockets.add(socket);
      socket.once('close', () => sockets.delete(socket));
    });
    await bounded(new Promise((resolveListen, reject) => {
      server.once('error', reject);
      server.listen(0, '127.0.0.1', resolveListen);
    }));
    return server.address().port;
  };
  const oldPort = await listen('old');
  const newPort = await listen('new');
  const origin = port => `http://127.0.0.1:${port}`;
  const profile = join(root, 'framerize-remote.json');
  const before = JSON.stringify({ remote: origin(oldPort),
    serverToken: { kind: 'value', value: oldToken }, localToken: { kind: 'value', value: localToken } }) + '\n';
  await writeFile(profile, before, { mode: 0o600 });
  const guard = join(root, 'guard.cjs');
  await writeFile(guard, `
const { appendFileSync } = require('node:fs');
const denied = () => {
  appendFileSync(${JSON.stringify(join(root, 'unexpected-command'))}, 'attempted\\n');
  throw new Error('Service and client commands are forbidden');
};
const commands = require('node:child_process');
for (const name of ['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync', 'fork'])
  commands[name] = denied;
require('node:module').syncBuiltinESMExports();
`, { mode: 0o600 });
  const target = origin(sample.same ? oldPort : newPort) + (sample.pair ? '/pair' : '');
  child = spawn(process.execPath, ['--require', guard, join(packageRoot, 'bin/frm.js'), 'up', target, '--json'], {
    cwd: root, stdio: ['ignore', 'pipe', 'pipe'],
    env: { HOME: root, USERPROFILE: root, FRM_HOME: root, PATH: '', XDG_CONFIG_HOME: root,
      TMPDIR: root, TMP: root, TEMP: root, NODE_DISABLE_COMPILE_CACHE: '1',
      ...(sample.explicit ? { FRM_SERVER_TOKEN: explicitToken } : {}) },
  });
  closed = new Promise((resolveClose, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => resolveClose({ code, signal }));
  });
  child.stdout.setEncoding('utf8').on('data', chunk => {
    stdout += chunk;
    if (stdout.length > 32768) child.kill('SIGKILL');
  });
  child.stderr.setEncoding('utf8').on('data', chunk => {
    stderr += chunk;
    if (stderr.length > 32768) child.kill('SIGKILL');
  });
  const exit = await bounded(closed);
  const restored = (await readFile(profile, 'utf8')) === before;
  const state = (await readdir(root)).sort();
  await cleanup();
  const ports = await Promise.all([oldPort, newPort].map(connectResult));
  const removed = await access(root).then(() => false, error => error.code === 'ENOENT');
  context.diagnostic(JSON.stringify({ name: sample.name, pid: child.pid, exit, seen,
    restored, state, ports, removed, sockets: sockets.size }));
  assert.deepEqual(exit, { code: 1, signal: null });
  assert.equal(stdout, '');
  assert.deepEqual(JSON.parse(stderr), { error: 'upstream_unavailable', code: 'upstream_unavailable',
    message: 'The remote upstream is unavailable.', next: 'Check the provider connection on the server.' });
  assert.equal(restored, true, 'Failed setup must restore the exact prior profile');
  assert.deepEqual(state, ['framerize-remote.json', 'guard.cjs']);
  assert.deepEqual(ports, ['ECONNREFUSED', 'ECONNREFUSED']);
  assert.equal(removed, true);
  assert.equal(sockets.size, 0);
  assert.deepEqual(seen, [{ side: sample.same ? 'old' : 'new', path: '/ready',
    authorization: sample.expected }], 'Only the credentials selected for this origin may reach it');
});
