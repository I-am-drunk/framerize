import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createConnection } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../framerize-remote/', import.meta.url)));
const launcher = join(packageRoot, 'bin/frm.js');

async function bounded(pending, message) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(message)), 5000);
    })]);
  } finally { clearTimeout(timer); }
}

async function readIfPresent(path) {
  try { return await readFile(path); }
  catch (error) {
    if (error.code === 'ENOENT') return undefined;
    throw error;
  }
}

function connect(port) {
  return new Promise((resolve, reject) => {
    const socket = createConnection({ host: '127.0.0.1', port });
    socket.setTimeout(2000, () => socket.destroy(new Error('Connection timed out')));
    socket.once('connect', () => { socket.destroy(); resolve(); });
    socket.once('error', reject);
  });
}

for (const command of ['up', 'connect']) for (const signal of ['SIGINT', 'SIGTERM']) {
  for (const existing of [false, true]) {
    test(`frm ${command} ${signal} restores ${existing ? 'an existing' : 'a new'} profile during readiness`,
      { timeout: 20000, skip: process.platform === 'win32'
        ? 'Windows child.kill terminates processes without delivering catchable POSIX signals' : false },
      async context => {
        const root = await mkdtemp(join(tmpdir(), 'frm-up-cancellation-'));
        const home = join(root, 'profile');
        const profile = join(home, 'framerize-remote.json');
        const commands = join(root, 'commands.jsonl');
        const guard = join(root, 'forbid-commands.cjs');
        const sockets = new Set();
        const requests = [];
        let child, closed, fixture;
        const running = () => child && child.exitCode === null && child.signalCode === null;
        context.after(async () => {
          try {
            if (running()) child.kill('SIGKILL');
            if (closed) await bounded(closed, 'Launcher cleanup timed out');
          } finally {
            for (const socket of sockets) socket.destroy();
            try {
              if (fixture?.listening) await bounded(new Promise(resolve => fixture.close(resolve)),
                'Fixture cleanup timed out');
            } finally { await rm(root, { recursive: true, force: true }); }
          }
        });
        await mkdir(home, { mode: 0o700 });
        await writeFile(guard, `
const { appendFileSync } = require('node:fs');
const processes = require('node:child_process');
for (const method of ['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync', 'fork']) {
  processes[method] = () => {
    appendFileSync(${JSON.stringify(commands)}, JSON.stringify({ method }) + '\\n');
    throw new Error('External commands are forbidden in this fixture');
  };
}
require('node:module').syncBuiltinESMExports();
`, { mode: 0o600 });
        let receive;
        const received = new Promise(resolve => { receive = resolve; });
        fixture = createServer(request => {
          requests.push({ method: request.method, path: request.url });
          request.resume();
          receive({ socket: request.socket, disconnected: once(request.socket, 'close') });
        });
        fixture.on('connection', socket => {
          sockets.add(socket);
          socket.once('close', () => sockets.delete(socket));
        });
        const listening = once(fixture, 'listening');
        fixture.listen(0, '127.0.0.1');
        await bounded(listening, 'Readiness fixture did not listen');
        const port = fixture.address().port;
        const origin = `http://127.0.0.1:${port}`;
        const original = Buffer.from(`{\r\n  "remote": "${origin}/",\r\n`
          + '  "host": "127.0.0.1", "port": 0\r\n}\r\n \t');
        if (existing) await writeFile(profile, original, { mode: 0o600 });
        child = spawn(process.execPath, ['--require', guard, launcher, command, origin, '--json'], {
          cwd: root, stdio: ['ignore', 'pipe', 'pipe'],
          env: { HOME: root, USERPROFILE: root, FRM_HOME: home, PATH: '',
            XDG_CONFIG_HOME: join(root, 'config'), XDG_DATA_HOME: join(root, 'data'),
            XDG_STATE_HOME: join(root, 'state'), XDG_CACHE_HOME: join(root, 'cache'),
            XDG_RUNTIME_DIR: join(root, 'run'), TMPDIR: root, TMP: root, TEMP: root,
            NODE_DISABLE_COMPILE_CACHE: '1' },
        });
        closed = new Promise((resolve, reject) => {
          child.once('error', reject);
          child.once('close', (code, signal) => resolve({ code, signal }));
        });
        let stdout = '', stderr = '';
        child.stdout.setEncoding('utf8').on('data', chunk => { stdout += chunk; });
        child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk; });
        const pending = await bounded(Promise.race([received, closed.then(result => {
          throw new Error(`Launcher exited before /ready: ${JSON.stringify(result)}`);
        })]), 'Launcher did not request /ready');
        const saved = await readFile(profile);
        assert.equal(JSON.parse(saved.toString()).remote, origin);
        if (existing) assert.notDeepEqual(saved, original, 'Profile must change before cancellation');
        assert.equal(pending.socket.destroyed, false, 'Signal must interrupt an open readiness request');
        assert.equal(child.kill(signal), true);
        const exit = await bounded(closed, 'Launcher did not settle after cancellation');
        await bounded(pending.disconnected, 'Readiness socket survived launcher cancellation');
        assert.equal(sockets.size, 0, 'Cancellation must release all readiness sockets');
        const restored = await readIfPresent(profile);
        const invoked = await readIfPresent(commands);
        const files = (await readdir(root, { recursive: true, withFileTypes: true }))
          .filter(entry => entry.isFile()).map(entry => join(entry.parentPath, entry.name)).sort();
        assert.deepEqual(files, [guard, ...(restored ? [profile] : [])].sort(),
          'No service unit, client configuration or command log may be created');
        await bounded(new Promise(resolve => fixture.close(resolve)), 'Fixture did not close');
        await assert.rejects(connect(port), { code: 'ECONNREFUSED' });
        const diagnostics = stderr.trim() ? stderr.trim().split('\n').map(line => {
          try {
            const record = JSON.parse(line);
            return { code: record?.code, error: record?.error };
          } catch { return { invalidJson: true }; }
        }) : [];
        const observed = { exit, diagnostics, stdout,
          successfulOutput: /"ready"\s*:\s*true|This machine/.test(stdout + stderr),
          profileRestored: existing ? original.equals(restored ?? Buffer.alloc(0)) : restored === undefined,
          externalCommands: invoked?.toString() ?? '', readinessSockets: sockets.size };
        context.diagnostic(JSON.stringify({ signal, existing, ...observed,
          fixturePortRefused: true, requests }));
        assert.deepEqual(requests, [{ method: 'GET', path: '/ready' }]);
        assert.deepEqual(observed, {
          exit: { code: 1, signal: null }, diagnostics: [{ code: 'aborted', error: 'aborted' }],
          stdout: '', successfulOutput: false, profileRestored: true,
          externalCommands: '', readinessSockets: 0,
        });
      });
  }
}
