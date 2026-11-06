import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { createServer, request } from 'node:http';
import { createConnection } from 'node:net';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const serverToken = 'synthetic-explicit-server';
const localToken = 'synthetic-explicit-local';
const ambientServer = 'synthetic-ambient-server';
const ambientLocal = 'synthetic-ambient-local';

async function bounded(pending) {
  let timer;
  try { return await Promise.race([pending, new Promise((unused, reject) => {
    timer = setTimeout(() => reject(new Error('Owned operation exceeded three seconds')), 3000);
  })]); } finally { clearTimeout(timer); }
}

function refused(port) {
  return new Promise(done => {
    const socket = createConnection({ host: '127.0.0.1', port });
    let result;
    const finish = value => { result = value; socket.destroy(); };
    socket.once('close', () => done(result));
    socket.once('connect', () => finish('connected'));
    socket.once('error', error => finish(error.code));
    socket.setTimeout(1000, () => finish('timeout'));
  });
}

function checkLocal(port, token) {
  return new Promise((resolveStatus, reject) => {
    const outgoing = request({ host: '127.0.0.1', port, path: '/ready', agent: false,
      headers: token ? { authorization: `Bearer ${token}` } : {} }, incoming => {
      incoming.resume();
      incoming.once('error', reject);
      incoming.once('end', () => resolveStatus(incoming.statusCode));
    });
    outgoing.once('error', reject);
    outgoing.setTimeout(2000, () => outgoing.destroy(new Error('Local readiness timed out')));
    outgoing.end();
  });
}

const cases = [
  { name: 'explicit setup token files override ambient token variables on --start', ambient: true, file: true },
  { name: 'explicit setup token env names override ambient token variables on --start', ambient: true, file: false },
  { name: 'explicit setup token files work without ambient token variables', ambient: false, file: true },
];

for (const sample of cases) test(sample.name, { timeout: 10000 }, async context => {
  const root = await mkdtemp(join(tmpdir(), 'frm-setup-token-fixture-'));
  const sockets = new Set(), seen = [], events = [];
  let child, closed, server, stdout = '', stderr = '', cleaned = false;
  const cleanup = async () => {
    if (cleaned) return;
    try {
      if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
      if (closed) await bounded(closed);
    } finally {
      const released = [...sockets].map(socket => new Promise(done => socket.once('close', done)));
      for (const socket of sockets) socket.destroy();
      await bounded(Promise.all(released));
      if (server) await bounded(new Promise(done => server.close(done)));
      await rm(root, { recursive: true, force: true });
      cleaned = true;
    }
  };
  context.after(cleanup);
  server = createServer((incoming, response) => {
    const auth = incoming.headers.authorization;
    const credential = auth === `Bearer ${serverToken}` ? 'explicit'
      : auth === `Bearer ${ambientServer}` ? 'ambient' : 'other';
    seen.push({ path: incoming.url, credential });
    incoming.resume();
    response.setHeader('content-type', 'application/json');
    response.end(JSON.stringify({ apiVersion: 1, status: 'ready',
      capabilities: { responses: true, chatCompletions: true, streaming: true } }));
  });
  server.on('connection', socket => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
  });
  await bounded(new Promise((done, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', done);
  }));
  const remotePort = server.address().port;
  const serverFile = join(root, 'server-token');
  const localFile = join(root, 'local-token');
  if (sample.file) {
    await writeFile(serverFile, `${serverToken}\n`, { mode: 0o600 });
    await writeFile(localFile, `${localToken}\n`, { mode: 0o600 });
  }
  const guard = join(root, 'guard.cjs');
  await writeFile(guard, `
const commands = require('node:child_process');
const denied = () => {
  require('node:fs').appendFileSync(${JSON.stringify(join(root, 'unexpected-command'))}, 'attempted\\n');
  throw new Error('Service and client commands are forbidden');
};
for (const name of ['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync', 'fork']) commands[name] = denied;
require('node:module').syncBuiltinESMExports();
`, { mode: 0o600 });
  let ready;
  const listening = new Promise(done => { ready = done; });
  const references = sample.file ? ['--server-token-file', serverFile, '--local-token-file', localFile]
    : ['--server-token-env', 'SELECTED_SERVER', '--local-token-env', 'SELECTED_LOCAL'];
  child = spawn(process.execPath, ['--require', guard, join(packageRoot, 'bin/frm.js'),
    'setup', '--start', '--json', '--host', '127.0.0.1', '--port', '0',
    '--remote', `http://127.0.0.1:${remotePort}`, ...references], {
    cwd: root, stdio: ['ignore', 'pipe', 'pipe'],
    env: { HOME: root, USERPROFILE: root, FRM_HOME: root, PATH: '', XDG_CONFIG_HOME: root,
      SELECTED_SERVER: serverToken, SELECTED_LOCAL: localToken,
      ...(sample.ambient ? { FRM_SERVER_TOKEN: ambientServer, FRM_LOCAL_TOKEN: ambientLocal } : {}),
      TMPDIR: root, TMP: root, TEMP: root, NODE_DISABLE_COMPILE_CACHE: '1' },
  });
  closed = new Promise((done, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => done({ code, signal }));
  });
  let pending = '';
  child.stdout.setEncoding('utf8').on('data', chunk => {
    stdout += chunk;
    pending += chunk;
    if (stdout.length > 32768) { child.kill('SIGKILL'); return; }
    const lines = pending.split('\n');
    pending = lines.pop();
    for (const line of lines) {
      try {
        const event = JSON.parse(line);
        events.push(event);
        if (event.listening) ready(event.listening.port);
      } catch { child.kill('SIGKILL'); }
    }
  });
  child.stderr.setEncoding('utf8').on('data', chunk => {
    stderr += chunk;
    if (stderr.length > 32768) child.kill('SIGKILL');
  });
  const port = await bounded(Promise.race([listening, closed.then(() => {
    throw new Error('Setup exited before its listener became ready');
  })]));
  const statuses = [];
  for (const token of [localToken, ambientLocal, undefined]) statuses.push(await bounded(checkLocal(port, token)));
  assert.equal(child.kill('SIGTERM'), true);
  const exit = await bounded(closed);
  const saved = JSON.parse(await readFile(join(root, 'framerize-remote.json'), 'utf8'));
  const referencesPreserved = JSON.stringify(saved.serverToken) === JSON.stringify(sample.file
    ? { kind: 'file', path: serverFile } : { kind: 'env', name: 'SELECTED_SERVER' })
    && JSON.stringify(saved.localToken) === JSON.stringify(sample.file
      ? { kind: 'file', path: localFile } : { kind: 'env', name: 'SELECTED_LOCAL' });
  const state = (await readdir(root)).sort();
  const tokenFilesUnchanged = !sample.file || (await readFile(serverFile, 'utf8')) === `${serverToken}\n`
    && (await readFile(localFile, 'utf8')) === `${localToken}\n`;
  const localClosed = await bounded(refused(port));
  await cleanup();
  const remoteClosed = await bounded(refused(remotePort));
  const removed = await readdir(root).then(() => false, error => error.code === 'ENOENT');
  context.diagnostic(JSON.stringify({ name: sample.name, pid: child.pid, exit, seen, statuses,
    referencesPreserved, tokenFilesUnchanged, state, localClosed, remoteClosed, removed, sockets: sockets.size }));
  if (process.platform === 'win32') assert.equal(exit.signal, 'SIGTERM');
  else assert.deepEqual(exit, { code: 0, signal: null });
  assert.equal(stderr, '');
  assert.equal(referencesPreserved, true);
  assert.equal(tokenFilesUnchanged, true);
  assert.deepEqual(state, ['framerize-remote.json', 'guard.cjs', ...(sample.file ? ['local-token', 'server-token'] : [])]);
  assert.equal(localClosed, 'ECONNREFUSED');
  assert.equal(remoteClosed, 'ECONNREFUSED');
  assert.equal(removed, true);
  assert.equal(sockets.size, 0);
  assert.equal(events[0].saved, true);
  assert.equal(events[1].ready, true);
  if (process.platform !== 'win32') assert.deepEqual(events.at(-1), { stopped: true });
  assert.deepEqual({ statuses, seen }, { statuses: [200, 401, 401], seen: [
    { path: '/ready', credential: 'explicit' }, { path: '/ready', credential: 'explicit' },
  ] }, 'Immediate setup startup must retain both explicit credential selections');
});
