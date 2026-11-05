import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { createConnection } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageRoot = process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT ?? fileURLToPath(new URL('../', import.meta.url));
const preload = fileURLToPath(new URL('./helpers/service-command-preload.mjs', import.meta.url));
async function bounded(promise, milliseconds = 4000) {
  let timer;
  try { return await Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error('Service fixture timed out')), milliseconds);
  })]); } finally { clearTimeout(timer); }
}
async function listening(port) {
  const socket = createConnection({ host: '127.0.0.1', port });
  try { return await bounded(new Promise((resolve, reject) => {
    socket.once('connect', () => resolve(true));
    socket.once('error', error => error.code === 'ECONNREFUSED' ? resolve(false) : reject(error));
  }), 1000); } finally { socket.destroy(); }
}

for (const failure of [false, true]) for (const command of ['service', 'up']) for (const signal of failure ? ['SIGTERM'] : ['SIGINT', 'SIGTERM']) {
  test(`${command} ${failure ? 'reports cleanup failure' : 'drains its service command group'} after ${signal}`, {
    timeout: 12000, skip: process.platform !== 'linux' && 'Uses a synthetic systemd runner',
  }, async t => {
    const root = await mkdtemp(join(tmpdir(), 'frm-service-cli-'));
    const home = join(root, 'profile'), config = join(home, 'framerize-remote.json');
    const server = createServer((request, response) => {
      request.resume();
      response.setHeader('content-type', 'application/json');
      response.end(JSON.stringify({ apiVersion: 1, status: 'ready',
        capabilities: { responses: true, chatCompletions: true, streaming: true } }));
    });
    let child, closed, address, stdout = '', stderr = '';
    const children = new Set();
    t.after(async () => {
      for (const pid of children) {
        try { process.kill(-pid, 'SIGKILL'); }
        catch (error) { if (error.code !== 'ESRCH') throw error; }
      }
      try {
        if (address) {
          try { process.kill(address.workerPid, 'SIGKILL'); }
          catch (error) { if (error.code !== 'ESRCH') throw error; }
        }
        if (child?.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
        if (closed) await bounded(closed);
        if (address) {
          const deadline = Date.now() + 1000;
          while (await listening(address.port) && Date.now() < deadline) {
            await new Promise(resolve => setTimeout(resolve, 10));
          }
          assert.equal(await listening(address.port), false);
        }
      } finally {
        server.closeAllConnections();
        await new Promise(resolve => server.close(resolve));
        await rm(root, { recursive: true, force: true });
      }
    });
    server.listen(0, '127.0.0.1');
    await once(server, 'listening');
    const origin = `http://127.0.0.1:${server.address().port}`;
    const before = `${JSON.stringify({ remote: origin, port: 8788 })}\n`;
    await mkdir(home);
    await writeFile(config, before, { mode: 0o600 });
    const args = command === 'up' ? ['up', origin, '--json'] : ['service', 'install', '--json'];
    child = spawn(process.execPath, ['--import', preload, join(packageRoot, 'bin/frm.js'), ...args], {
      cwd: root, env: { HOME: root, FRM_HOME: home, XDG_CONFIG_HOME: join(root, 'config'), PATH: '',
        ...(failure ? { FRM_FIXTURE_DENY_STOP: '1' } : {}) },
      stdio: ['ignore', 'pipe', 'pipe', 'ipc'],
    });
    const ready = Promise.withResolvers();
    child.on('message', message => {
      if (message.kind === 'child') {
        assert.ok(Number.isSafeInteger(message.pid) && message.pid > 1);
        children.add(message.pid);
      }
      if (message.kind === 'ready') { address = message; ready.resolve(message); }
    });
    child.stdout.on('data', chunk => { stdout += chunk; });
    child.stderr.on('data', chunk => { stderr += chunk; });
    child.once('error', ready.reject);
    closed = new Promise(resolve => child.once('close', (code, endedBy) => resolve({ code, endedBy })));
    await bounded(Promise.race([ready.promise, closed.then(result => {
      throw new Error(`Launcher exited before the command was ready: ${JSON.stringify(result)} ${stderr}`);
    })]));
    assert.ok(children.has(address.pid));
    assert.ok(Number.isSafeInteger(address.workerPid) && address.workerPid > 1);
    assert.equal(await listening(address.port), true);
    assert.equal(child.kill(signal), true);
    const result = await bounded(closed);
    const workerListening = await listening(address.port);
    t.diagnostic(JSON.stringify({ command, signal, result, workerListening }));
    assert.deepEqual(result, { code: 1, endedBy: null });
    assert.equal(workerListening, failure, 'the launcher must drain its command group or report cleanup failure');
    assert.equal(JSON.parse(stderr).error, failure ? 'service_cleanup_failed' : 'aborted');
    assert.doesNotMatch(stderr, /synthetic-private/);
    assert.equal(stdout, '');
    assert.equal(await readFile(config, 'utf8'), before);
    await assert.rejects(readFile(join(root, 'config/systemd/user/frm.service')), { code: 'ENOENT' });
  });
}
