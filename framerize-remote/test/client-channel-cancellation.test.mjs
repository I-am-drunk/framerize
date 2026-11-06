import assert from 'node:assert/strict';
import test from 'node:test';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, writeFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { getEventListeners } from 'node:events';
import { build } from 'esbuild';

const compiled = await build({ stdin: { contents:
  "export { runClient } from './src/compatibility/command.ts'; export { claudeUpdateChannel } from './src/compatibility/versions.ts';",
  resolveDir: fileURLToPath(new URL('../', import.meta.url)), loader: 'ts' },
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
const bundleHash = createHash('sha256').update(compiled.outputFiles[0].contents).digest('hex');
const sourceHashes = await Promise.all(Object.keys(compiled.metafile.inputs).filter(path => path !== '<stdin>').sort().map(async path =>
  [path, createHash('sha256').update(await readFile(path)).digest('hex')]));
const { runClient, claudeUpdateChannel } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

function deferred() {
  let resolve;
  const promise = new Promise(accept => { resolve = accept; });
  return { promise, resolve };
}

async function observe(pending, milliseconds = 1500) {
  let timer;
  try {
    return await Promise.race([pending, new Promise(resolve => {
      timer = setTimeout(() => resolve({ timedOut: true }), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function fixture(context, blockedIndex) {
  const state = { controller: new AbortController(), gate: deferred(), entered: deferred(),
    reads: [], lateReads: [], captures: 0, fetches: 0, runs: 0, output: [] };
  let cleanupPromise;
  state.finish = () => cleanupPromise ??= (async () => {
    state.gate.resolve();
    if (state.pending) assert.notEqual((await observe(state.pending)).timedOut, true, 'command settles after release');
    try {
      for (const [index, path] of (state.files ?? []).entries()) {
        assert.equal(await readFile(path, 'utf8'), state.originals[index], 'synthetic settings remain unchanged');
      }
    } finally {
      if (state.directory) {
        await rm(state.directory, { recursive: true, force: true });
        await assert.rejects(stat(state.directory), { code: 'ENOENT' });
      }
    }
  })();
  context.after(state.finish);
  state.directory = await mkdtemp(join(tmpdir(), 'frm-channel-cancellation-'));
  const home = join(state.directory, 'home');
  const bin = join(home, '.local', 'bin');
  const config = join(home, '.claude');
  await mkdir(bin, { recursive: true, mode: 0o700 });
  await mkdir(config, { recursive: true, mode: 0o700 });
  const executable = join(bin, 'claude');
  state.files = [join(config, 'settings.json'), join(state.directory, 'managed-settings.json')];
  state.originals = ['{"autoUpdatesChannel":"latest","fixture":"user"}\n',
    '{"autoUpdatesChannel":"stable","fixture":"managed"}\n'];
  for (const [index, path] of state.files.entries()) await writeFile(path, state.originals[index], { mode: 0o600 });
  const settingsPaths = [state.files[0], join('/etc/claude-code', 'managed-settings.json')];
  state.deps = { platform: 'linux', env: { HOME: home, PATH: bin }, home,
    cwd: state.directory, stateDir: join(state.directory, 'state'),
    target: { origin: 'http://127.0.0.1:8788' }, signal: state.controller.signal,
    probe: path => path === executable, realPath: path => path,
    io: { stdout: text => state.output.push(text), stderr: text => state.output.push(text) },
    async read(path) {
      const index = settingsPaths.indexOf(path);
      assert.notEqual(index, -1, 'only the two mapped private settings files may be read');
      state.reads.push(index);
      if (state.controller.signal.aborted) state.lateReads.push(index);
      const text = await readFile(state.files[index], 'utf8');
      if (index === blockedIndex) { state.entered.resolve({ entered: true }); await state.gate.promise; }
      return text;
    },
    async fetch(url, timeout, signal) {
      state.fetches += 1;
      assert.equal(url, 'https://downloads.claude.ai/claude-code-releases/stable');
      assert.equal(timeout, 4000);
      assert.equal(signal, state.controller.signal);
      assert.equal(signal.aborted, false, 'no release lookup after cancellation');
      return '1.0.0';
    },
    runner: {
      async capture(command, env, signal) {
        state.captures += 1;
        assert.equal(command.command, executable);
        assert.deepEqual(command.args, ['--version']);
        assert.equal(env.PATH, bin);
        assert.equal(signal, state.controller.signal);
        return '1.0.0 (Claude Code)';
      },
      async run() { state.runs += 1; throw new Error('Unexpected synthetic update or launch'); },
    },
  };
  state.start = () => {
    state.pending = runClient(['claude', '--update', '--no-input'], state.deps)
      .then(value => ({ value }), error => ({ error }));
    return state.pending;
  };
  return state;
}

for (const blockedIndex of [0, 1]) test(`native update cancels a held ${blockedIndex === 0 ? 'user' : 'managed'} channel read`,
  { timeout: 6000 }, async context => {
    const state = await fixture(context, blockedIndex);
    state.start();
    assert.deepEqual(await observe(state.entered.promise), { entered: true }, 'channel read is pending before abort');
    const reason = new Error('Synthetic update-channel cancellation');
    state.controller.abort(reason);
    const beforeRelease = await observe(state.pending, 1000);
    state.gate.resolve();
    const afterRelease = await observe(state.pending);
    const facts = { blockedIndex, settledBeforeRelease: beforeRelease.timedOut !== true,
      rejectionIsReason: afterRelease.error === reason, reads: state.reads, lateReads: state.lateReads,
      captures: state.captures, fetches: state.fetches, runs: state.runs, outputChunks: state.output.length };
    await state.finish();
    context.diagnostic(JSON.stringify({ ...facts, privateStateRemoved: true }));
    assert.equal(facts.rejectionIsReason, true, 'cancellation retains its reason');
    assert.equal(facts.captures, 1);
    assert.equal(facts.fetches, 0);
    assert.equal(facts.runs, 0);
    assert.equal(facts.outputChunks, 0, 'no update advice or success output follows cancellation');
    assert.deepEqual({ settled: facts.settledBeforeRelease, lateReads: facts.lateReads },
      { settled: true, lateReads: [] }, 'cancel before releasing the held reader, without later reads');
    assert.equal(beforeRelease.error, reason);
  });

test('uncancelled native update keeps managed channel precedence and current-version behavior',
  { timeout: 5000 }, async context => {
    const state = await fixture(context, undefined);
    const result = await observe(state.start());
    await state.finish();
    assert.deepEqual(result, { value: 0 });
    assert.deepEqual(state.reads, [0, 1]);
    assert.deepEqual(state.lateReads, []);
    assert.equal(state.captures, 1);
    assert.equal(state.fetches, 1);
    assert.equal(state.runs, 0);
    assert.match(state.output.join(''), /Version 1\.0\.0 is the latest\./);
    assert.doesNotMatch(state.output.join(''), /Update command:/);
    context.diagnostic('Synthetic native update control passed; private settings unchanged and removed.');
    context.diagnostic(JSON.stringify({ runtime: process.version, bundleHash, sourceHashes }));
  });

test('signal-less channel selection retains user and managed precedence', async () => {
  const cases = [
    ['latest', 'stable', 'stable'], ['stable', 'latest', 'latest'],
    ['stable', 'invalid', 'stable'], ['invalid', 'invalid', 'latest'],
  ];
  for (const [user, managed, expected] of cases) {
    const reads = [];
    const paths = ['/synthetic/settings.json', join('/etc/claude-code', 'managed-settings.json')];
    const actual = await claudeUpdateChannel(async path => {
      reads.push(path);
      assert.ok(paths.includes(path));
      return JSON.stringify({ autoUpdatesChannel: path === paths[0] ? user : managed });
    }, 'linux', {}, paths[0]);
    assert.equal(actual, expected);
    assert.deepEqual(reads, paths);
  }
});

test('pre-aborted channel selection reads nothing and retains the cancellation reason', async () => {
  const controller = new AbortController();
  const reason = new Error('Synthetic pre-aborted channel selection');
  controller.abort(reason);
  let reads = 0;
  await assert.rejects(claudeUpdateChannel(async () => { reads += 1; return null; },
    'linux', {}, '/synthetic/settings.json', controller.signal), error => error === reason);
  assert.equal(reads, 0);
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});
