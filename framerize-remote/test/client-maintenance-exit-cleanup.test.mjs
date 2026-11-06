import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { getEventListeners } from 'node:events';
import fs from 'node:fs';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { createConnection } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL(
  '../src/compatibility/process.ts', import.meta.url))], bundle: true,
  platform: 'node', format: 'esm', target: 'node22', write: false, metafile: true });
const bytes = compiled.outputFiles[0].contents;
console.log('maintenance source receipt', JSON.stringify({ node: process.version,
  bundle: createHash('sha256').update(bytes).digest('hex'), inputs: Object.keys(compiled.metafile.inputs) }));
const { nodeRunner } = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

function nonTerminal(context) {
  const original = fs.openSync;
  context.mock.method(fs, 'openSync', (path, ...args) => {
    if (path === '/dev/tty') throw new Error('Synthetic nonterminal command');
    return original(path, ...args);
  });
  syncBuiltinESMExports();
  context.after(() => { context.mock.restoreAll(); syncBuiltinESMExports(); });
}

async function bounded(work, milliseconds, message) {
  let timer;
  try {
    return await Promise.race([work, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(message)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function readState(path) {
  const deadline = performance.now() + 2000;
  do {
    try { return JSON.parse(await readFile(path, 'utf8')); }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
    await delay(10);
  } while (performance.now() < deadline);
  throw new Error('Owned descendant did not publish its listener');
}

function portState(port) {
  return new Promise(resolve => {
    const socket = createConnection({ host: '127.0.0.1', port });
    let settled = false;
    const finish = state => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      socket.destroy();
      resolve(state);
    };
    const timer = setTimeout(() => finish('timeout'), 300);
    socket.once('connect', () => finish('open'));
    socket.once('error', error => finish(error.code));
  });
}

async function portCloses(port) {
  const deadline = performance.now() + 1000;
  do {
    const state = await portState(port);
    if (state === 'ECONNREFUSED') return true;
    assert.equal(state, 'open');
    await delay(20);
  } while (performance.now() < deadline);
  return false;
}

function failingCommand(mode, statePath) {
  const descendant = `
const fs = require('node:fs');
const state = ${JSON.stringify(statePath)};
const server = require('node:net').createServer(socket => {
  socket.on('error', () => {});
  socket.end('synthetic maintenance worker');
  socket.once('close', () => {
    if (process.connected) process.send('verified', () => {});
  });
});
process.on('SIGTERM', () => {});
server.listen(0, '127.0.0.1', () => {
  fs.writeFileSync(state + '.tmp', JSON.stringify({ pid: process.pid, port: server.address().port }));
  fs.renameSync(state + '.tmp', state);
});
setTimeout(() => process.exit(90), 8000).unref();
`;
  return `
const child = require('node:child_process').spawn(process.execPath,
  ['-e', ${JSON.stringify(descendant)}], { stdio: ['ignore', 'ignore', 'ignore', 'ipc'] });
child.on('message', message => {
  if (message !== 'verified') return;
  child.disconnect();
  ${mode === 'nonzero' ? 'process.exit(7);' : "process.kill(process.pid, 'SIGTERM');"}
});
setTimeout(() => process.exit(91), 6000).unref();
`;
}

for (const mode of ['nonzero', 'signal']) {
  test(`failed maintenance run closes its descendant after ${mode} exit`, {
    skip: process.platform === 'win32', timeout: 6000,
  }, async context => {
    nonTerminal(context);
    const home = await mkdtemp(join(tmpdir(), 'framerize-maintenance-exit-home-'));
    const statePath = join(home, 'worker.json');
    const controller = new AbortController();
    let child, closed, state;
    context.after(async () => {
      try {
        if (child?.pid) {
          try { process.kill(-child.pid, 'SIGKILL'); }
          catch (error) { if (error.code !== 'ESRCH') throw error; }
        }
        if (closed) await bounded(closed, 1000, 'Owned command did not close during cleanup');
        if (state) assert.equal(await portCloses(state.port), true, 'Worker port remains open');
        console.log('owned maintenance fixture cleaned', JSON.stringify({ mode, ...state }));
      } finally { await rm(home, { recursive: true, force: true }); }
    });
    const runner = nodeRunner(process.platform, (...args) => {
      child = spawn(...args);
      closed = new Promise(resolve => child.once('close', resolve));
      return child;
    });
    const pending = runner.run({ command: process.execPath, args: ['-e', failingCommand(mode, statePath)],
      display: 'synthetic failed updater' }, { HOME: home, TMPDIR: home,
      PATH: dirname(process.execPath) }, controller.signal);
    void pending.catch(() => {});
    state = await readState(statePath);
    assert.ok(Number.isSafeInteger(state.pid) && state.pid > 1);
    assert.notEqual(state.pid, child.pid);
    assert.ok(Number.isInteger(state.port) && state.port > 0);
    assert.equal(await portState(state.port), 'open', 'Observe the live descendant before failure');
    const code = await bounded(pending, 2000, 'Failed maintenance command did not settle');
    await bounded(closed, 1000, 'Maintenance command did not close');
    assert.equal(code, mode === 'nonzero' ? 7 : 1);
    assert.equal(controller.signal.aborted, false, 'No cancellation was requested');
    assert.deepEqual(getEventListeners(controller.signal, 'abort'), []);
    assert.equal(child.exitCode, mode === 'nonzero' ? 7 : null);
    assert.equal(child.signalCode, mode === 'signal' ? 'SIGTERM' : null);
    const closedBeforeFixtureCleanup = await portCloses(state.port);
    console.log('failed maintenance outcome', JSON.stringify({ mode, ...state, code,
      exitCode: child.exitCode, signal: child.signalCode, closedBeforeFixtureCleanup }));
    assert.equal(closedBeforeFixtureCleanup, true, 'Failed maintenance must stop its owned descendants');
  });
}
