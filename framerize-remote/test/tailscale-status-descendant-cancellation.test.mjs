import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createConnection } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { build } from 'esbuild';

const source = fileURLToPath(new URL('../src/tailscale.ts', import.meta.url));
const compiled = await build({ entryPoints: [source], bundle: true, write: false, platform: 'node', format: 'esm' });
const bytes = compiled.outputFiles[0].contents;
const { readTailscaleStatus } = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
const digest = value => createHash('sha256').update(value).digest('hex');
console.log(JSON.stringify({ runtime: process.version, source: digest(await readFile(source)), bundle: digest(bytes) }));

async function bounded(promise, milliseconds) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Owned fixture exceeded its deadline')), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function connection(port) {
  return bounded(new Promise(resolve => {
    let result;
    const socket = createConnection({ host: '127.0.0.1', port });
    socket.once('connect', () => { result = 'connected'; socket.destroy(); });
    socket.once('error', error => { result = error.code; socket.destroy(); });
    socket.once('close', () => resolve(result));
    socket.setTimeout(500, () => socket.destroy(new Error('Probe connection timed out')));
  }), 750);
}

const wrapperSource = `
const { spawn } = require('node:child_process');
const { writeFileSync } = require('node:fs');
let worker;
process.on('SIGTERM', () => { worker?.kill('SIGKILL'); process.exit(0); });
worker = spawn(process.execPath, [process.argv[2]], { stdio: ['ignore', 'ignore', 'ignore', 'ipc'] });
writeFileSync(process.argv[3], String(worker.pid));
worker.on('message', message => process.stdout.write(JSON.stringify(message) + '\\n'));
setTimeout(() => { worker.kill('SIGKILL'); process.exit(0); }, 6000);
`;

const workerSource = `
const { createServer } = require('node:net');
const server = createServer(socket => socket.end());
process.on('SIGTERM', () => {});
server.listen(0, '127.0.0.1', () => process.send({ pid: process.pid, port: server.address().port }));
setTimeout(() => process.exit(0), 6000).unref();
`;

test('Tailscale status cancellation stops its owned descendant before returning', {
  skip: process.platform === 'win32', timeout: 7000,
}, async context => {
  const directory = await mkdtemp(join(tmpdir(), 'frm-tailscale-descendant-'));
  const wrapperPath = join(directory, 'wrapper.cjs');
  const workerPath = join(directory, 'worker.cjs');
  const pidPath = join(directory, 'worker.pid');
  const controller = new AbortController();
  let wrapper, wrapperClosed, pending, descendantPid, port, cleaned = false;
  async function cleanup() {
    if (cleaned) return;
    if (wrapper && wrapper.exitCode === null && wrapper.signalCode === null) wrapper.kill('SIGTERM');
    const recorded = await readFile(pidPath, 'utf8').catch(error => {
      if (error.code !== 'ENOENT') throw error;
      return '';
    });
    const pid = descendantPid ?? Number(recorded);
    if (Number.isInteger(pid) && pid > 0 && pid !== process.pid && pid !== wrapper?.pid) {
      try { process.kill(pid, 'SIGKILL'); }
      catch (error) { if (error.code !== 'ESRCH') throw error; }
    }
    if (wrapperClosed) {
      await bounded(wrapperClosed, 1000).catch(async () => {
        wrapper.kill('SIGKILL');
        await bounded(wrapperClosed, 500);
      });
    }
    if (pending) await bounded(pending, 1000);
    if (port) {
      const deadline = Date.now() + 1000;
      let state;
      do {
        state = await connection(port);
        if (state === 'ECONNREFUSED') break;
        await new Promise(resolve => setTimeout(resolve, 20));
      } while (Date.now() < deadline);
      assert.equal(state, 'ECONNREFUSED', 'owned descendant port closes during cleanup');
    }
    await rm(directory, { recursive: true, force: true });
    cleaned = true;
  }
  context.after(cleanup);
  await writeFile(wrapperPath, wrapperSource, { mode: 0o600 });
  await writeFile(workerPath, workerSource, { mode: 0o600 });
  let resolveReady, rejectReady;
  const ready = new Promise((resolve, reject) => { resolveReady = resolve; rejectReady = reject; });
  pending = readTailscaleStatus({ signal: controller.signal, env: { PATH: '' },
    launch(command, args, options) {
      assert.equal(command, 'tailscale');
      assert.deepEqual(args, ['status', '--json']);
      wrapper = spawn(process.execPath, [wrapperPath, workerPath, pidPath], options);
      wrapperClosed = new Promise(resolve => wrapper.once('close', (code, signal) => resolve({ code, signal })));
      wrapper.once('error', rejectReady);
      let output = '';
      wrapper.stdout.on('data', chunk => {
        output += chunk.toString('utf8');
        if (output.includes('\n')) {
          try { resolveReady(JSON.parse(output.split('\n', 1)[0])); }
          catch (error) { rejectReady(error); }
        }
      });
      return wrapper;
    },
  });
  const observed = await bounded(ready, 2000);
  descendantPid = observed.pid;
  port = observed.port;
  assert.ok(Number.isInteger(descendantPid) && descendantPid > 0 && descendantPid !== wrapper.pid);
  assert.ok(Number.isInteger(port) && port > 0 && port <= 65535);
  assert.equal(Number(await readFile(pidPath, 'utf8')), descendantPid);
  assert.equal(await connection(port), 'connected');
  controller.abort();
  const result = await bounded(pending, 3000);
  const wrapperExit = await bounded(wrapperClosed, 1000);
  const afterCancellation = await connection(port);
  await cleanup();
  console.log(JSON.stringify({ state: result.state, wrapperExit, afterCancellation, fixtureCleanup: cleaned }));
  assert.equal(result.state, 'cancelled');
  assert.equal(afterCancellation, 'ECONNREFUSED', 'cancellation must close the descendant listener before returning');
});
