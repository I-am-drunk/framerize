import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, mkdir, readFile, writeFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { getEventListeners } from 'node:events';
import { createHash } from 'node:crypto';
import { build } from 'esbuild';

const compiled = await build({ stdin: { contents:
  "export { runClient } from './src/compatibility/command.ts'; export { inspectClaudeSettings } from './src/compatibility/conflicts.ts';",
resolveDir: fileURLToPath(new URL('../', import.meta.url)), loader: 'ts' },
bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
const bundleHash = createHash('sha256').update(compiled.outputFiles[0].contents).digest('hex');
const sourceHashes = await Promise.all(Object.keys(compiled.metafile.inputs)
  .filter(path => path !== '<stdin>').sort().map(async path => [path,
    createHash('sha256').update(await readFile(path)).digest('hex')]));
const { runClient, inspectClaudeSettings } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

function deferred() {
  let resolve;
  const promise = new Promise(accept => { resolve = accept; });
  return { promise, resolve };
}

async function observe(pending, milliseconds = 2000) {
  let timer;
  try {
    return await Promise.race([pending, new Promise(resolve => {
      timer = setTimeout(() => resolve({ timedOut: true }), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

test('normal client preparation cancels a held conflict-settings read',
  { timeout: 6000, skip: process.platform === 'win32' }, async context => {
    const held = deferred();
    const entered = deferred();
    let directory, pending, cleanupPromise;
    const cleanup = () => cleanupPromise ??= (async () => {
      held.resolve();
      if (pending) assert.notEqual((await observe(pending)).timedOut, true);
      if (directory) await rm(directory, { recursive: true, force: true });
    })();
    context.after(cleanup);
    directory = await mkdtemp(join(tmpdir(), 'framerize-conflict-read-state-'));
    const home = join(directory, 'home');
    const cwd = join(directory, 'project');
    const bin = join(directory, 'bin');
    const executable = join(bin, 'claude');
    const userPath = join(home, '.claude', 'settings.json');
    const paths = [userPath, join(cwd, '.claude', 'settings.json'),
      join(cwd, '.claude', 'settings.local.json'), '/etc/claude-code/managed-settings.json'];
    await mkdir(join(home, '.claude'), { recursive: true, mode: 0o700 });
    const original = '{"env":{},"synthetic_marker":"unchanged"}\n';
    await writeFile(userPath, original, { mode: 0o600 });
    const target = { origin: 'http://127.0.0.1:8788' };
    const makeDeps = (controller, hold) => {
      const calls = { reads: [], readsAfterAbort: [], captures: 0, launches: 0, output: [] };
      return { calls, deps: { platform: 'linux', env: { HOME: home, PATH: bin }, home, cwd,
        stateDir: join(directory, 'state'), target, signal: controller.signal,
        probe: path => path === executable, realPath: path => path,
        io: { stdout: text => calls.output.push(text), stderr: text => calls.output.push(text) },
        async fetch() { throw new Error('Unexpected release fetch'); },
        async read(path) {
          assert.ok(paths.includes(path), 'Only synthetic settings paths are requested');
          calls.reads.push(paths.indexOf(path));
          if (controller.signal.aborted) calls.readsAfterAbort.push(paths.indexOf(path));
          if (path !== userPath) return null;
          const text = await readFile(userPath, 'utf8');
          if (hold) { entered.resolve(); await held.promise; }
          return text;
        },
        runner: {
          async capture(command) {
            assert.equal(command.command, 'git');
            assert.deepEqual(command.args, ['-C', cwd, 'worktree', 'list', '--porcelain', '-z']);
            assert.equal(controller.signal.aborted, false, 'No new command after cancellation');
            calls.captures += 1;
            return null;
          },
          async run(command) {
            assert.equal(command.command, executable);
            assert.equal(controller.signal.aborted, false, 'Cancelled preparation must not launch');
            calls.launches += 1;
            return 0;
          },
        },
      } };
    };
    const control = makeDeps(new AbortController(), false);
    assert.equal(await runClient(['claude'], control.deps), 0);
    assert.equal(control.calls.captures, 1);
    assert.equal(control.calls.launches, 1);
    assert.deepEqual(control.calls.reads, [0, 1, 2, 3]);
    const controller = new AbortController();
    const cancelled = makeDeps(controller, true);
    const reason = new Error('Synthetic preparation cancellation');
    pending = runClient(['claude'], cancelled.deps).then(
      code => ({ code }), error => ({ error }));
    assert.notEqual((await observe(entered.promise.then(() => ({})))).timedOut, true);
    controller.abort(reason);
    const early = await observe(pending, 500);
    const readsBeforeRelease = cancelled.calls.reads.length;
    held.resolve();
    const result = await observe(pending);
    assert.equal(result.error, reason);
    assert.equal(cancelled.calls.captures, 1);
    assert.equal(cancelled.calls.launches, 0);
    assert.deepEqual(cancelled.calls.output, []);
    assert.equal(await readFile(userPath, 'utf8'), original);
    await cleanup();
    await assert.rejects(stat(directory), { code: 'ENOENT' });
    context.diagnostic(JSON.stringify({ runtime: process.version, bundleHash, sourceHashes,
      control: { code: 0, reads: control.calls.reads, launches: control.calls.launches },
      settledBeforeRelease: early.timedOut !== true, readsBeforeRelease,
      readsAfterAbort: cancelled.calls.readsAfterAbort, cancelledLaunches: cancelled.calls.launches,
      reasonPreserved: result.error === reason, outputAfterCancellation: cancelled.calls.output,
      settingsUnchanged: true, privateStateRemoved: true }));
    assert.notEqual(early.timedOut, true, 'Cancellation must settle before the settings read is released');
    assert.deepEqual(cancelled.calls.readsAfterAbort, [], 'No later settings read after cancellation');
  });

const inspection = signal => ({ platform: 'linux', env: {},
  userPath: '/synthetic-home/.claude/settings.json', cwd: '/synthetic-project',
  expected: { ANTHROPIC_BASE_URL: 'http://127.0.0.1:8788/anthropic' },
  ...(signal ? { signal } : {}) });

test('pre-aborted conflict inspection does not start a reader', async () => {
  const controller = new AbortController();
  const reason = new Error('Synthetic pre-abort');
  controller.abort(reason);
  let reads = 0;
  await assert.rejects(inspectClaudeSettings(async () => { reads += 1; return null; },
    inspection(controller.signal)), error => error === reason);
  assert.equal(reads, 0);
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});

test('reader errors retain their identity and release the abort listener', async () => {
  const controller = new AbortController();
  const reason = new Error('Synthetic read failure');
  let reads = 0;
  await assert.rejects(inspectClaudeSettings(async () => { reads += 1; throw reason; },
    inspection(controller.signal)), error => error === reason);
  assert.equal(reads, 1);
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});

test('conflict inspection without a signal preserves a foreign URL override', async () => {
  const input = inspection();
  const paths = [];
  const result = await inspectClaudeSettings(async path => {
    paths.push(path);
    return path === input.userPath
      ? JSON.stringify({ env: { ANTHROPIC_BASE_URL: 'https://synthetic.invalid' } }) : null;
  }, input);
  assert.deepEqual(result, { overrideBaseUrl: true, blocking: [] });
  assert.equal(paths.length, 4);
});

test('aborted inspection releases its listener and handles late rejection',
  { timeout: 4000 }, async context => {
    const controller = new AbortController();
    const entered = deferred();
    const read = Promise.withResolvers();
    const reason = new Error('Synthetic pending-read cancellation');
    let reads = 0;
    let pending;
    context.after(async () => {
      controller.abort(reason);
      if (reads) read.reject(new Error('Synthetic late reader failure'));
      else read.resolve(null);
      if (pending) assert.notEqual((await observe(pending)).timedOut, true);
    });
    pending = inspectClaudeSettings(() => {
      reads += 1;
      entered.resolve();
      return read.promise;
    }, inspection(controller.signal)).then(value => ({ value }), error => ({ error }));
    assert.notEqual((await observe(entered.promise.then(() => ({})))).timedOut, true);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 1);
    controller.abort(reason);
    const outcome = await observe(pending);
    assert.equal(outcome.error, reason);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    assert.equal(reads, 1);
    read.reject(new Error('Synthetic late reader failure'));
    await new Promise(resolve => setImmediate(resolve));
  });
