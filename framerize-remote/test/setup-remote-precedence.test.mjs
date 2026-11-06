import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { createServer, request } from 'node:http';
import { createConnection } from 'node:net';
import { access, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));

async function bounded(pending) {
  let timer;
  try { return await Promise.race([pending, new Promise((unused, reject) => {
    timer = setTimeout(() => reject(new Error('Owned operation exceeded three seconds')), 3000);
  })]); } finally { clearTimeout(timer); }
}

function connectResult(port) {
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

const cases = [
  { name: 'setup --start uses its explicit remote instead of FRM_REMOTE', explicit: true, start: true },
  { name: 'setup --start uses FRM_REMOTE without an explicit remote', explicit: false, start: true },
  { name: 'setup without --start saves the explicit remote without contacting either origin', explicit: true, start: false },
];

function post(port) {
  return new Promise((resolveReply, reject) => {
    const outgoing = request({ host: '127.0.0.1', port, path: '/v1/responses', method: 'POST',
      agent: false, headers: { 'content-type': 'application/json' } }, incoming => {
      let body = '';
      incoming.setEncoding('utf8').on('data', chunk => { body += chunk; });
      incoming.once('error', reject);
      incoming.once('end', () => resolveReply({ status: incoming.statusCode, body }));
    });
    outgoing.once('error', reject);
    outgoing.setTimeout(2000, () => outgoing.destroy(new Error('Fixture request timed out')));
    outgoing.end('{"model":"synthetic","input":"probe","stream":false}');
  });
}

for (const sample of cases) test(sample.name, { timeout: 10000 }, async context => {
  const root = await mkdtemp(join(tmpdir(), 'frm-setup-remote-fixture-'));
  const servers = [], sockets = new Set(), seen = [], events = [];
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
    const server = createServer((incoming, response) => {
      seen.push({ side, path: incoming.url, authorization: incoming.headers.authorization ?? null });
      incoming.resume();
      response.setHeader('content-type', 'application/json');
      response.end(JSON.stringify(incoming.url === '/ready'
        ? { apiVersion: 1, status: 'ready', capabilities: { responses: true, chatCompletions: true, streaming: true } }
        : { side }));
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
  const ambientPort = await listen('environment');
  const explicitPort = await listen('explicit');
  const origin = port => `http://127.0.0.1:${port}`;
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
  child = spawn(process.execPath, ['--require', guard, join(packageRoot, 'bin/frm.js'),
    'setup', '--json', '--host', '127.0.0.1', '--port', '0',
    ...(sample.explicit ? ['--remote', origin(explicitPort)] : []), ...(sample.start ? ['--start'] : [])], {
    cwd: root, stdio: ['ignore', 'pipe', 'pipe'],
    env: { HOME: root, USERPROFILE: root, FRM_HOME: root, PATH: '', XDG_CONFIG_HOME: root,
      FRM_REMOTE: origin(ambientPort), TMPDIR: root, TMP: root, TEMP: root, NODE_DISABLE_COMPILE_CACHE: '1' },
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
  let localPort, reply;
  if (sample.start) {
    localPort = await bounded(Promise.race([listening, closed.then(() => {
      throw new Error('The setup launcher exited before listening');
    })]));
    reply = await bounded(post(localPort));
    assert.equal(child.kill('SIGTERM'), true);
  }
  const exit = await bounded(closed);
  const saved = JSON.parse(await readFile(join(root, 'framerize-remote.json'), 'utf8'));
  const savedSelectedRemote = saved.remote === origin(sample.explicit ? explicitPort : ambientPort);
  const state = (await readdir(root)).sort();
  const localClosed = localPort === undefined ? undefined : await bounded(connectResult(localPort));
  await cleanup();
  const fixtureClosed = await bounded(Promise.all([ambientPort, explicitPort].map(connectResult)));
  const removed = await access(root).then(() => false, error => error.code === 'ENOENT');
  context.diagnostic(JSON.stringify({ name: sample.name, pid: child.pid, exit, seen, reply,
    savedSelectedRemote, state, localClosed, fixtureClosed, removed, sockets: sockets.size }));
  if (sample.start && process.platform === 'win32') assert.equal(exit.signal, 'SIGTERM');
  else assert.deepEqual(exit, { code: 0, signal: null });
  assert.equal(stderr, '');
  assert.equal(savedSelectedRemote, true);
  assert.equal(saved.port, 0);
  assert.equal(saved.serverToken, undefined);
  assert.equal(saved.localToken, undefined);
  assert.deepEqual(state, ['framerize-remote.json', 'guard.cjs']);
  assert.deepEqual(fixtureClosed, ['ECONNREFUSED', 'ECONNREFUSED']);
  assert.equal(removed, true);
  assert.equal(sockets.size, 0);
  assert.equal(events[0].saved, true);
  if (!sample.start) { assert.deepEqual(events, [{ saved: true }]); assert.deepEqual(seen, []); return; }
  assert.equal(localClosed, 'ECONNREFUSED');
  assert.equal(events[1].ready, true);
  if (process.platform !== 'win32') assert.deepEqual(events.at(-1), { stopped: true });
  const side = sample.explicit ? 'explicit' : 'environment';
  assert.deepEqual(seen, ['/ready', '/ready', '/v1/responses'].map(path => ({ side, path, authorization: null })),
    'Immediate startup must use the selected remote, not replace an explicit flag with the ambient origin');
  assert.deepEqual(reply, { status: 200, body: JSON.stringify({ side }) });
});
