import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, rm } from 'node:fs/promises';
import { createConnection } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL(
  '../src/compatibility/process.ts', import.meta.url))], bundle: true,
  platform: 'node', format: 'esm', target: 'node22', write: false, metafile: true });
const source = compiled.outputFiles[0].contents;
console.log('capture source receipt', JSON.stringify({ node: process.version,
  bundle: createHash('sha256').update(source).digest('hex'), inputs: Object.keys(compiled.metafile.inputs) }));
const { nodeRunner } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);

async function bounded(work, milliseconds, description) {
  let timer;
  try {
    return await Promise.race([work, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(description)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

function stopOwnedGroup(child) {
  if (!child?.pid) return;
  try { process.kill(-child.pid, 'SIGKILL'); }
  catch (error) { if (error.code !== 'ESRCH') throw error; }
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

const descendantSource = `
const server = require('node:net').createServer(socket => {
  socket.on('error', () => {});
  socket.end('synthetic listener');
  socket.once('close', () => {
    if (process.connected) process.send({ kind: 'verified' }, () => {});
  });
});
server.listen(0, '127.0.0.1', () => {
  process.send({ kind: 'ready', pid: process.pid, port: server.address().port });
});
setTimeout(() => process.exit(90), 8000).unref();
`;

function failingProbe(mode) {
  return `
const descendant = require('node:child_process').spawn(process.execPath,
  ['-e', ${JSON.stringify(descendantSource)}], { stdio: ['ignore', 'ignore', 'ignore', 'ipc'] });
descendant.on('message', message => {
  if (message.kind === 'ready') process.stdout.write(JSON.stringify(message) + '\\n');
  if (message.kind === 'verified') {
    descendant.disconnect();
    ${mode === 'nonzero' ? 'process.exit(7);' : "process.kill(process.pid, 'SIGTERM');"}
  }
});
setTimeout(() => process.exit(91), 6000).unref();
`;
}

for (const mode of ['nonzero', 'signal']) {
  test(`failed capture closes separate-stdio descendants after ${mode} exit`, {
    skip: process.platform === 'win32', timeout: 6000,
  }, async context => {
    const home = await mkdtemp(join(tmpdir(), 'framerize-capture-exit-home-'));
    let child, closed, ready, metadata;
    context.after(async () => {
      try {
        stopOwnedGroup(child);
        if (closed) await bounded(closed, 1000, 'owned probe did not close during cleanup');
        if (metadata) assert.equal(await portCloses(metadata.port), true, 'fixture port remains open');
        console.log('owned capture fixture cleaned', JSON.stringify({ mode, ...metadata }));
      } finally { await rm(home, { recursive: true, force: true }); }
    });
    const runner = nodeRunner(process.platform, (...args) => {
      child = spawn(...args);
      closed = new Promise(resolve => child.once('close', resolve));
      ready = new Promise((resolve, reject) => {
        let output = '';
        child.stdout.on('data', chunk => {
          output += chunk.toString('utf8');
          if (!output.includes('\n')) return;
          try { resolve(JSON.parse(output.split('\n', 1)[0])); }
          catch (error) { reject(error); }
        });
      });
      return child;
    });
    const pending = runner.capture({ command: process.execPath, args: ['-e', failingProbe(mode)],
      display: 'synthetic failed version probe' }, { HOME: home, TMPDIR: home,
      PATH: dirname(process.execPath) });
    metadata = await bounded(ready, 2000, 'descendant did not report its listener');
    assert.equal(metadata.kind, 'ready');
    assert.ok(Number.isSafeInteger(metadata.pid) && metadata.pid > 1);
    assert.notEqual(metadata.pid, child.pid);
    assert.ok(Number.isInteger(metadata.port) && metadata.port > 0);
    assert.equal(await portState(metadata.port), 'open', 'descendant must be observed alive');
    assert.equal(await bounded(pending, 2000, 'failed capture did not settle'), null);
    await bounded(closed, 1000, 'probe did not close');
    assert.equal(child.exitCode, mode === 'nonzero' ? 7 : null);
    assert.equal(child.signalCode, mode === 'signal' ? 'SIGTERM' : null);
    const closedBeforeFixtureCleanup = await portCloses(metadata.port);
    console.log('failed capture outcome', JSON.stringify({ mode, ...metadata,
      exitCode: child.exitCode, signal: child.signalCode, closedBeforeFixtureCleanup }));
    assert.equal(closedBeforeFixtureCleanup, true, 'failed capture must stop its owned descendants');
  });
}

for (const code of [0, 7]) {
  test(`capture keeps ordinary exit ${code} behavior`, {
    skip: process.platform === 'win32', timeout: 5000,
  }, async context => {
    const home = await mkdtemp(join(tmpdir(), 'framerize-capture-control-'));
    let child, closed;
    context.after(async () => {
      try {
        stopOwnedGroup(child);
        if (closed) await bounded(closed, 1000, 'control probe did not close');
      } finally { await rm(home, { recursive: true, force: true }); }
    });
    const runner = nodeRunner(process.platform, (...args) => {
      child = spawn(...args);
      closed = new Promise(resolve => child.once('close', resolve));
      return child;
    });
    const result = await bounded(runner.capture({ command: process.execPath,
      args: ['-e', `process.stdout.write('1.2.3\\n', () => process.exit(${code}));`],
      display: 'synthetic ordinary version probe' }, { HOME: home, TMPDIR: home,
      PATH: dirname(process.execPath) }), 2000, 'ordinary capture did not settle');
    assert.equal(result, code === 0 ? '1.2.3\n' : null);
    assert.equal(child.exitCode, code);
  });
}
