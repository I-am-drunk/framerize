import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { getEventListeners } from 'node:events';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/tailscale.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
const bundleHash = createHash('sha256').update(compiled.outputFiles[0].contents).digest('hex');
const sourceHashes = await Promise.all(Object.keys(compiled.metafile.inputs).sort().map(async path =>
  [path, createHash('sha256').update(await readFile(path)).digest('hex')]));
const { readTailscaleStatus } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function bounded(pending) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolveUnused, reject) => {
      timer = setTimeout(() => reject(new Error('Owned status fixture exceeded its bound')), 3500);
    })]);
  } finally { clearTimeout(timer); }
}

function fixture(context, script) {
  const state = { controller: new AbortController() };
  const originalKill = process.kill;
  let resolveReady;
  state.ready = new Promise(resolve => { resolveReady = resolve; });
  context.after(async () => {
    process.kill = originalKill;
    if (state.child && state.child.exitCode === null && state.child.signalCode === null) {
      state.kill('SIGKILL');
    }
    if (state.closed) await bounded(state.closed);
    if (state.pending) await bounded(state.pending.catch(() => {}));
    context.diagnostic('Captured synthetic child closed; process.kill restored.');
  });
  state.start = platform => {
    state.pending = readTailscaleStatus({
      platform, signal: state.controller.signal, env: { PATH: '' },
      launch(command, args, options) {
        assert.equal(command, platform === 'win32' ? 'tailscale.exe' : 'tailscale');
        assert.deepEqual(args, ['status', '--json']);
        assert.equal(options.detached, platform !== 'win32');
        state.child = spawn(process.execPath, ['-e', script], options);
        state.kill = state.child.kill.bind(state.child);
        state.closed = new Promise(resolve => state.child.once('close', resolve));
        state.child.stdout.once('data', resolveReady);
        return state.child;
      },
    });
    return state.pending;
  };
  state.originalKill = originalKill;
  return state;
}

test('successful status retains parsing and launches an owned POSIX group', {
  timeout: 5000, skip: process.platform === 'win32',
}, async context => {
  const status = { BackendState: 'Running', Self: { TailscaleIPs: ['100.101.2.3'] } };
  const state = fixture(context, 'process.stdout.write(' + JSON.stringify(JSON.stringify(status)) + ');');
  const result = await bounded(state.start('linux'));
  assert.equal(result.state, 'running');
  assert.equal(state.child.exitCode, 0);
  assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
  context.diagnostic(JSON.stringify({ runtime: process.version, bundleHash, sourceHashes }));
});

for (const mode of ['timeout', 'output_limit']) test(`status ${mode} closes its owned command`, {
  timeout: 5000, skip: process.platform === 'win32',
}, async context => {
  const output = mode === 'output_limit' ? '"x".repeat(300000)' : '"pending"';
  const state = fixture(context, 'process.stdout.write(' + output + '); setTimeout(() => process.exit(0), 5000);');
  const result = await bounded(state.start('linux'));
  await bounded(state.closed);
  assert.equal(result.state, mode);
  assert.equal(state.child.signalCode, 'SIGKILL');
  assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
  context.diagnostic('Captured command closed before status returned.');
});

test('a denied group signal reports cleanup failure instead of successful cancellation', {
  timeout: 5000, skip: process.platform === 'win32',
}, async context => {
  const state = fixture(context, 'process.stdout.write("ready"); setTimeout(() => process.exit(0), 5000);');
  state.start('linux');
  await bounded(state.ready);
  const failure = Object.assign(new Error('Synthetic owned-group signal denial'), { code: 'EACCES' });
  process.kill = (pid, signal) => {
    if (pid === -state.child.pid && signal === 'SIGKILL') throw failure;
    return state.originalKill(pid, signal);
  };
  const rejected = assert.rejects(state.pending, error =>
    error.code === 'service_cleanup_failed' && error.cause === failure);
  state.controller.abort();
  await bounded(rejected);
  assert.equal(state.child.exitCode, null);
  assert.equal(state.child.signalCode, null);
  assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
  context.diagnostic('Denied cleanup was reported while the captured child was still alive; fixture cleanup owns its stop.');
});

test('a simulated Windows stop refusal reports cleanup failure', {
  timeout: 5000, skip: process.platform === 'win32',
}, async context => {
  const state = fixture(context, 'process.stdout.write("ready"); setTimeout(() => process.exit(0), 5000);');
  state.start('win32');
  await bounded(state.ready);
  state.child.kill = () => false;
  const rejected = assert.rejects(state.pending, error =>
    error.code === 'service_cleanup_failed' && /could not be stopped/.test(error.cause?.message));
  state.controller.abort();
  await bounded(rejected);
  assert.equal(state.child.exitCode, null);
  assert.equal(state.child.signalCode, null);
  assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
  context.diagnostic('Windows branch simulated on POSIX; this is not native Windows evidence.');
});

for (const completion of ['code', 'signal']) test(`an exited simulated Windows command tolerates cancellation after ${completion}`, {
  timeout: 5000, skip: process.platform === 'win32',
}, async context => {
  const script = completion === 'code' ? 'process.stdout.write("ready");'
    : 'process.stdout.write("ready"); setTimeout(() => process.exit(0), 5000);';
  const state = fixture(context, script);
  state.start('win32');
  state.child.kill = () => false;
  state.child.once('exit', () => state.controller.abort());
  if (completion === 'signal') {
    await bounded(state.ready);
    state.kill('SIGTERM');
  }
  const result = await bounded(state.pending);
  assert.equal(result.state, 'cancelled');
  await bounded(state.closed);
  assert.equal(completion === 'code' ? state.child.exitCode : state.child.signalCode,
    completion === 'code' ? 0 : 'SIGTERM');
  assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
});
