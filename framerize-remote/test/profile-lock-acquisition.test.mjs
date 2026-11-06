import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build, stop } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const compiled = await build({ absWorkingDir: root, stdin: {
  contents: "export { withProfileLock } from './profileLock.ts'; export { setUp } from './cmdUp.ts';",
  resolveDir: join(root, 'framerize-remote/src'), loader: 'ts',
}, alias: { '@framerize/core': join(root, 'framerize-core/src/index.ts') },
bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
stop();
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
console.log(JSON.stringify({ runtime: process.version, bundle: hash(compiled.outputFiles[0].contents),
  inputs: Object.fromEntries(await Promise.all(Object.keys(compiled.metafile.inputs)
    .filter(path => path !== '<stdin>').map(async path => [path, hash(await fs.readFile(join(root, path)))]))) }));
const { withProfileLock, setUp } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function bounded(promise) {
  let timer;
  try { return await Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error('Profile lock fixture timed out')), 1500);
  })]); } finally { clearTimeout(timer); }
}

async function fixture(context) {
  const directory = await fs.realpath(await fs.mkdtemp(join(tmpdir(), 'frm-lock-acquisition-')));
  const state = { directory, path: join(directory, 'profile.json'),
    lock: join(directory, '.profile.json.lock'), controller: new AbortController(),
    reason: new Error('Synthetic lock cancellation'), release: Promise.withResolvers(),
    entered: Promise.withResolvers(), held: undefined, pending: undefined,
    handles: [], closes: 0, writes: 0, operations: 0 };
  state.bytes = '{"remote":"http://127.0.0.1:8789","port":8788}\n';
  await fs.writeFile(state.path, state.bytes, { mode: 0o600 });
  context.after(async () => {
    state.controller.abort(state.reason);
    state.release.resolve();
    try {
      if (state.held) await bounded(state.held.catch(() => {}));
      if (state.pending) await bounded(state.pending);
    } finally {
      context.mock.restoreAll();
      syncBuiltinESMExports();
      for (const handle of state.handles) if (handle.fd >= 0) await handle.close();
      await fs.rm(directory, { recursive: true, force: true });
      await assert.rejects(fs.access(directory), { code: 'ENOENT' });
    }
  });
  return state;
}

async function cleanState(state) {
  assert.equal(await fs.readFile(state.path, 'utf8'), state.bytes);
  assert.deepEqual(await fs.readdir(state.directory), ['profile.json']);
  assert.equal(state.handles.filter(handle => handle.fd >= 0).length, 0);
  assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
}

function hold(state, pending) {
  state.held = pending.then(async value => {
    state.entered.resolve();
    await state.release.promise;
    return value;
  });
  return state.held;
}

function trackHandle(context, state, handle, phase) {
  state.handles.push(handle);
  const close = handle.close.bind(handle), write = handle.writeFile.bind(handle);
  context.mock.method(handle, 'close', async () => {
    state.closes++;
    await close();
    if (phase === 'close-error') throw new Error('Synthetic close failure');
  });
  context.mock.method(handle, 'writeFile', async (...args) => {
    state.writes++;
    assert.equal(args[1].signal, state.controller.signal);
    await hold(state, write(...args));
    if (phase === 'write-error') throw new Error('Synthetic late write failure');
  });
  return handle;
}

for (const phase of ['mkdir', 'open', 'write', 'write-error', 'close-error']) {
  test(`cancellation during lock ${phase} preserves ownership and skips the operation`,
    { timeout: 5000 }, async context => {
      const state = await fixture(context);
      const mkdir = fs.mkdir.bind(fs), open = fs.open.bind(fs);
      context.mock.method(fs, 'mkdir', (...args) => {
        const pending = mkdir(...args);
        return phase === 'mkdir' && args[0] === state.directory ? hold(state, pending) : pending;
      });
      context.mock.method(fs, 'open', async (...args) => {
        const handle = await open(...args);
        if (args[0] !== state.lock) return handle;
        trackHandle(context, state, handle, phase);
        return phase === 'open' || phase === 'close-error'
          ? hold(state, Promise.resolve(handle)) : handle;
      });
      syncBuiltinESMExports();
      state.pending = withProfileLock(state.path, async () => { state.operations++; },
        state.controller.signal).then(value => ({ value }), error => ({ error }));
      await bounded(state.entered.promise);
      state.controller.abort(state.reason);
      if (phase === 'mkdir') assert.equal((await bounded(state.pending)).error, state.reason);
      state.release.resolve();
      const result = await bounded(state.pending);
      await state.held;
      if (phase === 'close-error') {
        assert.equal(result.error?.code, 'profile_lock_cleanup');
        assert.equal(result.error.original, state.reason);
      } else assert.equal(result.error, state.reason);
      assert.equal(state.operations, 0);
      assert.equal(state.closes, state.handles.length);
      if (phase === 'open' || phase === 'close-error') assert.equal(state.writes, 0);
      await cleanState(state);
    });
}

test('a pre-aborted acquisition starts no filesystem work', async context => {
  const state = await fixture(context);
  let creates = 0;
  context.mock.method(fs, 'mkdir', () => { creates++; throw new Error('Unexpected mkdir'); });
  syncBuiltinESMExports();
  state.controller.abort(state.reason);
  await assert.rejects(withProfileLock(state.path, async () => { state.operations++; },
    state.controller.signal), error => error === state.reason);
  assert.equal(creates, 0);
  assert.equal(state.operations, 0);
  await cleanState(state);
});

test('cancellation after acquisition keeps the lock until the operation completes', async context => {
  const state = await fixture(context);
  state.pending = withProfileLock(state.path, async () => {
    state.entered.resolve();
    await state.release.promise;
    return 42;
  }, state.controller.signal).then(value => ({ value }), error => ({ error }));
  await bounded(state.entered.promise);
  state.controller.abort(state.reason);
  assert.equal(await fs.readFile(state.lock, 'utf8'), `${process.pid}\n`);
  await assert.rejects(withProfileLock(state.path, async () => {}), { code: 'profile_busy' });
  state.release.resolve();
  assert.equal((await bounded(state.pending)).value, 42);
  await cleanState(state);
});

test('cancellation takes priority over a late busy result without removing its lock', async context => {
  const state = await fixture(context);
  await fs.writeFile(state.lock, 'another writer\n', { mode: 0o600 });
  const open = fs.open.bind(fs);
  context.mock.method(fs, 'open', async (...args) => {
    if (args[0] !== state.lock) return open(...args);
    const error = await open(...args).then(() => assert.fail('The lock must exist'), error => error);
    throw await hold(state, Promise.resolve(error));
  });
  syncBuiltinESMExports();
  state.pending = withProfileLock(state.path, async () => { state.operations++; },
    state.controller.signal).then(value => ({ value }), error => ({ error }));
  await bounded(state.entered.promise);
  state.controller.abort(state.reason);
  state.release.resolve();
  assert.equal((await bounded(state.pending)).error, state.reason);
  assert.equal(state.operations, 0);
  assert.equal(await fs.readFile(state.lock, 'utf8'), 'another writer\n');
  await fs.rm(state.lock);
  await cleanState(state);
});

for (const source of ['options', 'service']) {
  test(`up forwards its ${source} signal before taking the profile lock`, async context => {
    const state = await fixture(context);
    const realpath = fs.realpath.bind(fs);
    context.mock.method(fs, 'realpath', (...args) => args[0] === state.directory
      ? hold(state, realpath(...args)) : realpath(...args));
    syncBuiltinESMExports();
    const output = [], commands = [];
    const options = { configPath: state.path, env: {}, service: {
      home: state.directory, host: { platform: 'win32', homeDir: state.directory },
      program: ['synthetic-frm'], environmentPath: '',
      run: async (...args) => { commands.push(args); throw new Error('Unexpected service call'); },
    } };
    if (source === 'options') options.signal = state.controller.signal;
    else options.service.signal = state.controller.signal;
    state.pending = setUp('http://127.0.0.1:9876', {
      out: text => output.push(text), err: text => output.push(text),
    }, options).then(value => ({ value }), error => ({ error }));
    await bounded(state.entered.promise);
    state.controller.abort(state.reason);
    assert.equal((await bounded(state.pending)).error?.code, 'aborted');
    state.release.resolve();
    await state.held;
    assert.deepEqual(output, []);
    assert.deepEqual(commands, []);
    await cleanState(state);
  });
}
