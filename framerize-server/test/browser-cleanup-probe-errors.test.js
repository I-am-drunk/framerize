import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { once, getEventListeners } from 'node:events';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { createConnection } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/browser-login.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
const bundleHash = createHash('sha256').update(compiled.outputFiles[0].contents).digest('hex');
const sourceHashes = await Promise.all(Object.keys(compiled.metafile.inputs).sort().map(async path =>
  [path, createHash('sha256').update(await readFile(path)).digest('hex')]));
const { loginWithBrowser } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function bounded(pending, milliseconds = 6000) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolveUnused, reject) => {
      timer = setTimeout(() => reject(new Error('Owned browser fixture exceeded its bound')), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function portState(port) {
  const socket = createConnection({ host: '127.0.0.1', port });
  try {
    return await bounded(new Promise((resolve, reject) => {
      socket.once('connect', () => resolve('open'));
      socket.once('error', error => error.code === 'ECONNREFUSED'
        ? resolve(error.code) : reject(error));
    }), 1000);
  } finally { socket.destroy(); }
}

const listenerCode = `
const net = require('node:net');
const fs = require('node:fs');
const server = net.createServer(socket => socket.on('error', () => {}));
server.listen(0, '127.0.0.1', () => {
  const info = { pid: process.pid, port: server.address().port };
  if (process.send) process.send(info);
  else fs.writeSync(5, JSON.stringify(info) + '\\n');
});
process.on('disconnect', () => {});
`;
const leaderCode = `
const { spawn } = require('node:child_process');
const fs = require('node:fs');
const worker = spawn(process.execPath, ['-e', ${JSON.stringify(listenerCode)}], {
  stdio: ['ignore', 'ignore', 'ignore', 'ipc']
});
worker.once('message', info => fs.writeSync(5, JSON.stringify(info) + '\\n'));
`;

for (const inspectionError of ['EIO', 'ESRCH']) {
  test(`browser cleanup does not mistake ${inspectionError} for confirmed absence`, {
    timeout: 12_000, skip: process.platform === 'win32',
  }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'browser-probe-error-'));
    const controller = new AbortController();
    let child, childClosed, outcome, info, profile, heldSocket, heldClosed;
    let inspections = 0, cleaned = false;
    const signals = [];
    const stop = async () => {
      if (cleaned) return;
      controller.abort();
      if (child?.pid) {
        try { process.kill(-child.pid, 'SIGKILL'); }
        catch (error) { if (error.code !== 'ESRCH') throw error; }
      }
      if (childClosed) await bounded(childClosed);
      if (heldClosed) await bounded(heldClosed);
      if (outcome) await bounded(outcome);
      if (info) assert.equal(await portState(info.port), 'ECONNREFUSED');
      await rm(root, { recursive: true, force: true });
      cleaned = true;
    };
    context.after(async () => {
      try { await stop(); } finally { heldSocket?.destroy(); }
    });
    const fixturePath = join(root, 'fixture.cjs');
    await writeFile(fixturePath, inspectionError === 'EIO' ? leaderCode : listenerCode);
    let readyResolve, readyReject;
    const ready = new Promise((resolve, reject) => { readyResolve = resolve; readyReject = reject; });
    outcome = loginWithBrowser({ interactive: true, platform: 'linux',
      browser: process.execPath, env: { DISPLAY: ':synthetic', HOME: root },
      temporaryDirectory: root, signal: controller.signal, timeoutMs: 8000,
      framerOrigin: 'http://127.0.0.1:1',
      notice() {}, confirm: signal => new Promise((resolve, reject) => {
        if (signal.aborted) reject(signal.reason);
        else signal.addEventListener('abort', () => reject(signal.reason), { once: true });
      }),
      spawn(command, args, options) {
        assert.equal(command, process.execPath);
        profile = args.find(value => value.startsWith('--user-data-dir=')).slice(16);
        child = spawn(process.execPath, [fixturePath], { ...options,
          stdio: [...options.stdio, 'pipe'], cwd: root, env: { HOME: root, TMPDIR: root } });
        childClosed = new Promise(resolve => child.once('close', resolve));
        child.once('error', readyReject);
        let output = '';
        child.stdio[5].on('data', chunk => {
          output += chunk.toString();
          if (output.includes('\n')) {
            try { info = JSON.parse(output.trim()); readyResolve(info); }
            catch (error) { readyReject(error); }
          }
        });
        return child;
      },
      kill(pid, signal) {
        assert.equal(pid === -child.pid || pid === child.pid, true);
        if (signal === 0) {
          inspections++;
          if (inspectionError === 'EIO') {
            throw Object.assign(new Error('synthetic group inspection failure'), { code: 'EIO' });
          }
        } else signals.push(signal);
        process.kill(pid, signal);
      },
    }).then(value => ({ value }), error => ({ error }));
    await bounded(Promise.race([ready, outcome.then(() => {
      throw new Error('Login settled before the owned fixture was ready');
    })]));
    assert.equal(await portState(info.port), 'open');
    heldSocket = createConnection({ host: '127.0.0.1', port: info.port });
    heldSocket.on('error', () => {});
    heldClosed = new Promise(resolve => heldSocket.once('close', resolve));
    await bounded(once(heldSocket, 'connect'));
    child.kill('SIGTERM');
    const result = await bounded(outcome);
    const portAfterResult = await portState(info.port);
    const removed = await stat(profile).then(() => false, error => error.code === 'ENOENT');
    const abortListeners = getEventListeners(controller.signal, 'abort').length;
    await stop();
    context.diagnostic(JSON.stringify({ runtime: process.version, inspectionError,
      bundleHash, sourceHashes, code: result.error?.code, inspections, signals,
      portAfterResult, profileRemoved: removed, abortListeners, cleanupPort: 'ECONNREFUSED' }));
    assert.equal(result.error?.code, inspectionError === 'EIO' ? 'cleanup_failed' : 'browser_closed');
    assert.equal(portAfterResult, 'ECONNREFUSED');
    assert.equal(removed, true);
    assert.equal(abortListeners, 0);
    assert.ok(inspections > 0);
    if (inspectionError === 'EIO') assert.ok(signals.includes('SIGKILL'));
    else assert.deepEqual(signals, []);
  });
}
