import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { createServer } from 'node:http';
import { connect } from 'node:net';
import { access, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
async function bounded(pending, milliseconds = 2500) {
  let timer;
  try { return await Promise.race([pending, new Promise((unused, reject) => {
    timer = setTimeout(() => reject(new Error('Owned operation timed out')), milliseconds);
  })]); } finally { clearTimeout(timer); }
}
async function refused(port) {
  const socket = connect({ host: '127.0.0.1', port });
  try { return await bounded(new Promise(resolveResult => {
    let result;
    socket.once('connect', () => { result = 'connected'; socket.destroy(); });
    socket.once('error', error => { result = error.code; socket.destroy(); });
    socket.once('close', () => resolveResult(result));
  })); } finally { socket.destroy(); }
}

const cases = [[false, undefined, 'setup'], [true, undefined, 'setup'],
  [true, 'SIGHUP', 'setup'], [true, 'SIGINT', 'up'], [true, 'SIGTERM', 'up']];
for (const [existing, signal, contender] of cases) test(`separate ${contender} rejects a busy ${existing ? 'existing' : 'absent'} profile and retries after ${signal ?? 'failure'}`,
  { timeout: 12000, skip: signal && process.platform === 'win32'
    ? 'Windows does not deliver catchable POSIX signals' : false }, async context => {
    const home = await mkdtemp(join(tmpdir(), 'frm-cross-process-'));
    const profile = join(home, 'framerize-remote.json');
    const children = [], servers = [], sockets = new Set(), requests = [];
    let enter, held, cleaned = false;
    const entered = new Promise(resolveEntered => { enter = resolveEntered; });
    const cleanup = async () => {
      if (cleaned) return;
      try {
        held?.destroy();
        for (const entry of children) {
          if (entry.child.exitCode === null && entry.child.signalCode === null) entry.child.kill('SIGKILL');
        }
        await bounded(Promise.all(children.map(entry => entry.closed)));
      } finally {
        for (const socket of sockets) socket.destroy();
        await bounded(Promise.all(servers.map(server => new Promise(done => server.close(done)))));
        await rm(home, { recursive: true, force: true });
        cleaned = true;
      }
    };
    context.after(cleanup);
    const listen = async side => {
      const server = createServer((request, response) => {
        requests.push({ side, path: request.url });
        if (side === 'older' && !held) { held = response; enter(); }
        else { response.writeHead(500); response.end(); }
      });
      servers.push(server);
      server.on('connection', socket => {
        sockets.add(socket);
        socket.once('close', () => sockets.delete(socket));
      });
      await bounded(new Promise((done, reject) => {
        server.once('error', reject); server.listen(0, '127.0.0.1', done);
      }));
      return server.address().port;
    };
    const olderPort = await listen('older'), newerPort = await listen('newer');
    const origin = port => `http://127.0.0.1:${port}`;
    const before = '{"remote":"http://127.0.0.1:1","port":8788}\n';
    if (existing) await writeFile(profile, before, { mode: 0o600 });
    const marker = join(home, 'unexpected-command'), guard = join(home, 'guard.cjs');
    await writeFile(guard, `
const commands = require('node:child_process');
const deny = () => {
  require('node:fs').appendFileSync(${JSON.stringify(marker)}, 'attempted\\n');
  throw new Error('Service and client commands are forbidden');
};
for (const name of ['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync', 'fork']) commands[name] = deny;
require('node:module').syncBuiltinESMExports();
`, { mode: 0o600 });
    const launch = args => {
      const child = spawn(process.execPath, ['--require', guard, join(packageRoot, 'bin/frm.js'), ...args], {
        cwd: home, stdio: ['ignore', 'pipe', 'pipe'],
        env: { HOME: home, USERPROFILE: home, FRM_HOME: home, XDG_CONFIG_HOME: home,
          PATH: '', TMPDIR: home, TMP: home, TEMP: home, NODE_DISABLE_COMPILE_CACHE: '1' },
      });
      const entry = { child, stdout: '', stderr: '' };
      entry.closed = new Promise((done, reject) => {
        child.once('error', reject);
        child.once('close', (code, signal) => done({ code, signal }));
      });
      void entry.closed.catch(() => {});
      for (const channel of ['stdout', 'stderr']) child[channel].setEncoding('utf8').on('data', chunk => {
        entry[channel] += chunk;
        if (entry[channel].length > 32768) child.kill('SIGKILL');
      });
      children.push(entry);
      return entry;
    };
    const older = launch(['up', origin(olderPort), '--json']);
    await bounded(entered);
    const waiting = await readFile(profile, 'utf8');
    const newer = launch(contender === 'up' ? ['up', origin(newerPort), '--json']
      : ['setup', '--remote', origin(newerPort), '--json']);
    const newerExit = await bounded(newer.closed);
    assert.deepEqual(newerExit, { code: 1, signal: null });
    assert.equal(newer.stdout, '');
    assert.equal(JSON.parse(newer.stderr).code, 'profile_busy');
    assert.equal(await readFile(profile, 'utf8'), waiting);
    if (signal) assert.equal(older.child.kill(signal), true);
    else {
      held.writeHead(503, { 'content-type': 'application/json' });
      held.end(JSON.stringify({ apiVersion: 1, status: 'unavailable', reason: 'upstream_unavailable',
        capabilities: { responses: true, chatCompletions: true, streaming: true } }));
    }
    const olderExit = await bounded(older.closed);
    const restored = await readFile(profile, 'utf8').catch(error => {
      if (error.code === 'ENOENT') return undefined;
      throw error;
    });
    assert.equal(restored, existing ? before : undefined);
    await assert.rejects(access(join(home, '.framerize-remote.json.lock')), { code: 'ENOENT' });
    const retry = launch(['setup', '--remote', origin(newerPort), '--json']);
    const retryExit = await bounded(retry.closed);
    assert.deepEqual(retryExit, { code: 0, signal: null });
    assert.deepEqual(JSON.parse(retry.stdout), { saved: true });
    assert.equal(retry.stderr, '');
    const committed = await readFile(profile, 'utf8');
    assert.equal(JSON.parse(committed).remote, origin(newerPort));
    await assert.rejects(access(join(home, '.framerize-remote.json.lock')), { code: 'ENOENT' });
    const after = await readFile(profile, 'utf8');
    const unexpectedCommand = await access(marker).then(() => true, error => error.code !== 'ENOENT');
    await cleanup();
    const ports = await Promise.all([olderPort, newerPort].map(refused));
    const removed = await access(home).then(() => false, error => error.code === 'ENOENT');
    context.diagnostic(JSON.stringify({ olderPid: older.child.pid, newerPid: newer.child.pid,
      existing, signal, contender, olderExit, newerExit, retryExit, requests, committedSha256: digest(committed), afterSha256: digest(after),
      restoredStaleBytes: after === before, unexpectedCommand, ports, removed, sockets: sockets.size }));
    assert.deepEqual(olderExit, { code: 1, signal: null });
    assert.equal(older.stdout, '');
    assert.equal(JSON.parse(older.stderr).code, signal ? 'aborted' : 'upstream_unavailable');
    assert.deepEqual(requests, [{ side: 'older', path: '/ready' }]);
    assert.equal(unexpectedCommand, false);
    assert.deepEqual(ports, ['ECONNREFUSED', 'ECONNREFUSED']);
    assert.equal(removed, true);
    assert.equal(sockets.size, 0);
    assert.equal(after, committed, 'The retry must keep its saved profile');
  });
