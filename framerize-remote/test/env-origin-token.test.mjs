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
const savedToken = 'synthetic-saved-origin-token';
const explicitToken = 'synthetic-destination-token';
const localToken = 'synthetic-local-token';
const classify = value => value === undefined ? 'none' : value === `Bearer ${savedToken}`
  ? 'saved' : value === `Bearer ${explicitToken}` ? 'explicit'
    : value === `Bearer ${localToken}` ? 'local' : 'other';

async function bounded(pending) {
  let timer;
  try { return await Promise.race([pending, new Promise((unused, reject) => {
    timer = setTimeout(() => reject(new Error('Owned operation exceeded three seconds')), 3000);
  })]); } finally { clearTimeout(timer); }
}

const cases = [
  { name: 'check without FRM_REMOTE retains the saved token', expected: 'saved' },
  { name: 'check with the same FRM_REMOTE retains the saved token', override: 'same', expected: 'saved' },
  { name: 'check with a different FRM_REMOTE does not forward the saved token', override: 'new', expected: 'none' },
  { name: 'check with a different FRM_REMOTE uses its explicit token', override: 'new', explicit: true, expected: 'explicit' },
];

function connectResult(port) {
  return new Promise(resolveResult => {
    const socket = createConnection({ host: '127.0.0.1', port });
    let result;
    const finish = value => { result = value; socket.destroy(); };
    socket.once('close', () => resolveResult(result));
    socket.once('connect', () => finish('connected'));
    socket.once('error', error => finish(error.code));
    socket.setTimeout(1000, () => finish('timeout'));
  });
}

for (const sample of cases) test(sample.name, { timeout: 8000 }, async context => {
  const root = await mkdtemp(join(tmpdir(), 'frm-env-origin-fixture-'));
  const servers = [], sockets = new Set(), seen = [];
  let child, closed, stdout = '', stderr = '', cleaned = false;
  const cleanup = async () => {
    if (cleaned) return;
    try {
      if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
      if (closed) await bounded(closed);
    } finally {
      const released = [...sockets].map(socket => new Promise(done => socket.once('close', done)));
      for (const socket of sockets) socket.destroy();
      await bounded(Promise.all(released));
      await bounded(Promise.all(servers.map(server => new Promise(done => server.close(done)))));
      await rm(root, { recursive: true, force: true });
      cleaned = true;
    }
  };
  context.after(cleanup);
  const listen = async side => {
    const server = createServer((request, response) => {
      seen.push({ side, path: request.url, authorization: classify(request.headers.authorization) });
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ apiVersion: 1, status: 'ready',
        capabilities: { responses: true, chatCompletions: true, streaming: true } }));
    });
    servers.push(server);
    server.on('connection', socket => {
      sockets.add(socket);
      socket.once('close', () => sockets.delete(socket));
    });
    await bounded(new Promise((done, reject) => {
      server.once('error', reject);
      server.listen(0, '127.0.0.1', done);
    }));
    return server.address().port;
  };
  const oldPort = await listen('saved');
  const newPort = await listen('new');
  const origin = port => `http://127.0.0.1:${port}`;
  const profile = join(root, 'framerize-remote.json');
  const before = JSON.stringify({ remote: origin(oldPort),
    serverToken: { kind: 'value', value: savedToken }, localToken: { kind: 'value', value: localToken } }) + '\n';
  await writeFile(profile, before, { mode: 0o600 });
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
  child = spawn(process.execPath, ['--require', guard, join(packageRoot, 'bin/frm.js'), 'check', '--json'], {
    cwd: root, stdio: ['ignore', 'pipe', 'pipe'],
    env: { HOME: root, USERPROFILE: root, FRM_HOME: root, PATH: '', XDG_CONFIG_HOME: root,
      TMPDIR: root, TMP: root, TEMP: root, NODE_DISABLE_COMPILE_CACHE: '1',
      ...(sample.override ? { FRM_REMOTE: origin(sample.override === 'same' ? oldPort : newPort) } : {}),
      ...(sample.explicit ? { FRM_SERVER_TOKEN: explicitToken } : {}) },
  });
  closed = new Promise((done, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => done({ code, signal }));
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
  const unchanged = (await readFile(profile, 'utf8')) === before;
  const state = (await readdir(root)).sort();
  await cleanup();
  const ports = await bounded(Promise.all([oldPort, newPort].map(connectResult)));
  const removed = await access(root).then(() => false, error => error.code === 'ENOENT');
  context.diagnostic(JSON.stringify({ name: sample.name, pid: child.pid, exit, seen,
    unchanged, state, ports, removed, sockets: sockets.size }));
  assert.deepEqual(exit, { code: 0, signal: null });
  assert.equal(stderr, '');
  assert.deepEqual(JSON.parse(stdout), { ready: true });
  assert.equal(unchanged, true);
  assert.deepEqual(state, ['framerize-remote.json', 'guard.cjs']);
  assert.deepEqual(ports, ['ECONNREFUSED', 'ECONNREFUSED']);
  assert.equal(removed, true);
  assert.equal(sockets.size, 0);
  assert.deepEqual(seen, [{ side: sample.override === 'new' ? 'new' : 'saved', path: '/ready',
    authorization: sample.expected }], 'An origin override must not reuse another origin\'s saved credential');
});
