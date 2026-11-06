import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { EventEmitter } from 'node:events';
import { createHash } from 'node:crypto';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/cli.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
  plugins: [{ name: 'source-boundaries', setup(builder) {
    builder.onResolve({ filter: /^\.\/(backend|setup|account-cli|browser-login)\.js$/ },
      args => ({ path: args.path, external: true }));
    builder.onResolve({ filter: /\/dist\/core\.js$/ }, () => ({
      path: fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)),
    }));
  } }],
});
stop();
assert.ok(Object.keys(compiled.metafile.inputs).every(path => !/castle|backend\.ts|browser-login/.test(path)));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
console.log(JSON.stringify({ runtime: process.version, bundle: hash(compiled.outputFiles[0].contents),
  inputs: await Promise.all(Object.keys(compiled.metafile.inputs).sort().map(async path =>
    [path, hash(await fs.readFile(path))])) }));
const { run } = await import(`data:text/javascript;base64,${
  Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function bounded(promise, milliseconds = 1000) {
  let timer;
  try {
    return await Promise.race([promise, new Promise(resolve => {
      timer = setTimeout(() => resolve({ timedOut: true }), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function fixture(context) {
  const root = await fs.mkdtemp(join(tmpdir(), 'startup-settings-cancel-'));
  const events = new EventEmitter();
  const state = { root, events, output: '', records: [], calls: [], release() {}, pending: undefined };
  context.after(async () => {
    state.release();
    events.emit('SIGTERM');
    const result = state.pending ? await bounded(state.pending) : {};
    context.mock.restoreAll();
    syncBuiltinESMExports();
    await fs.rm(root, { recursive: true, force: true });
    await assert.rejects(fs.stat(root), { code: 'ENOENT' });
    assert.notEqual(result.timedOut, true, 'Owned startup must settle before cleanup');
  });
  state.path = join(root, 'server.json');
  state.bytes = Buffer.from(JSON.stringify({ version: 1, environment: {}, config: {
    backend: 'http', upstream: 'http://127.0.0.1:9', host: '127.0.0.1', port: 0,
  } }));
  await fs.writeFile(state.path, state.bytes, { mode: 0o600 });
  events.stdout = events.stderr = { write(text) { state.output += text; } };
  state.start = (command, explicit = true) => {
    state.pending = run([command, ...(explicit ? ['--config', state.path] : [])], {
      process: events, env: { HOME: root, FRAMERIZE_SERVER_HOME: root },
      logger: { info(event) {
        state.records.push(event);
        if (event === 'server.listening') events.emit('SIGTERM');
      } },
      createProvider() { state.calls.push('provider'); return {}; },
      createServer() {
        state.calls.push('server');
        return {
          async listen() { state.calls.push('listen'); return { port: 8789 }; },
          async close() { state.calls.push('close'); },
        };
      },
    }).then(value => ({ value }), error => ({ error }));
    return state.pending;
  };
  return state;
}

for (const command of ['check', 'serve']) {
  for (const signal of ['SIGINT', 'SIGTERM']) {
    test(`${command} observes ${signal} during settings loading`, { timeout: 5000 }, async context => {
      const state = await fixture(context);
      const originalOpen = fs.open;
      const entered = Promise.withResolvers(), released = Promise.withResolvers();
      const closed = Promise.withResolvers();
      let closes = 0;
      state.release = () => released.resolve();
      context.mock.method(fs, 'open', async function (path, ...options) {
        const handle = await originalOpen.call(this, path, ...options);
        if (String(path) !== state.path) return handle;
        const read = handle.readFile.bind(handle), close = handle.close.bind(handle);
        context.mock.method(handle, 'readFile', async (...args) => {
          const bytes = await read(...args);
          entered.resolve({ entered: true });
          await released.promise;
          return bytes;
        });
        context.mock.method(handle, 'close', async () => {
          closes++;
          try { return await close(); } finally { closed.resolve(); }
        });
        return handle;
      });
      syncBuiltinESMExports();
      const pending = state.start(command, signal === 'SIGINT');
      assert.equal((await bounded(Promise.race([entered.promise, pending]))).entered, true);
      const handlersBeforeSignal = ['SIGINT', 'SIGTERM'].map(name => state.events.listenerCount(name));
      state.events.emit(signal);
      const beforeRelease = await bounded(pending);
      released.resolve();
      const afterRelease = await bounded(pending);
      assert.notEqual((await bounded(closed.promise))?.timedOut, true);
      context.diagnostic(JSON.stringify({ command, signal, handlersBeforeSignal,
        stoppedBeforeRelease: beforeRelease.value === 0, afterCode: afterRelease.value,
        calls: state.calls, records: state.records, closes }));
      assert.equal(closes, 1);
      assert.equal(state.output, '');
      assert.deepEqual(await fs.readFile(state.path), state.bytes);
      for (const name of ['SIGINT', 'SIGTERM']) assert.equal(state.events.listenerCount(name), 0);
      assert.deepEqual(handlersBeforeSignal, [1, 1]);
      assert.equal(beforeRelease.value, 0, 'Startup must stop before the held read completes');
      assert.equal(afterRelease.value, 0);
      assert.deepEqual(state.calls, [], 'Cancelled settings load must not construct a provider or server');
      assert.deepEqual(state.records, []);
    });
  }

  test(`${command} removes startup handlers after settings failure`, async context => {
    const state = await fixture(context);
    await fs.writeFile(state.path, '{', { mode: 0o600 });
    const result = await state.start(command);
    assert.equal(result.error?.message, 'Server settings are unavailable or invalid');
    assert.deepEqual(state.calls, []);
    assert.deepEqual(state.records, []);
    for (const name of ['SIGINT', 'SIGTERM']) assert.equal(state.events.listenerCount(name), 0);
  });

  test(`${command} retains normal startup and shutdown`, async context => {
    const state = await fixture(context);
    assert.equal((await state.start(command)).value, 0);
    assert.deepEqual(state.calls, command === 'check' ? ['provider', 'server', 'close']
      : ['provider', 'server', 'listen', 'close']);
    assert.deepEqual(state.records, [command === 'check' ? 'config.valid' : 'server.listening']);
    for (const name of ['SIGINT', 'SIGTERM']) assert.equal(state.events.listenerCount(name), 0);
  });
}
