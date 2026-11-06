import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { EventEmitter } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
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
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const inputs = Object.keys(compiled.metafile.inputs).sort();
assert.ok(inputs.every(path => !/castle|backend\.ts|browser-login/.test(path)));
console.log(JSON.stringify({ runtime: process.version, bundle: hash(compiled.outputFiles[0].contents),
  inputs: await Promise.all(inputs.map(async path => [path, hash(await fs.readFile(path))])) }));
const { run } = await import(`data:text/javascript;base64,${
  Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function bounded(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise(resolve => {
      timer = setTimeout(() => resolve({ timedOut: true }), 1000);
    })]);
  } finally { clearTimeout(timer); }
}

async function fixture(context) {
  const root = await fs.mkdtemp(join(tmpdir(), 'default-settings-cancel-'));
  const events = new EventEmitter();
  const state = { events, path: join(root, 'server.json'), calls: [], records: [],
    output: '', release() {}, pending: undefined };
  context.after(async () => {
    state.release();
    events.emit('SIGTERM');
    const result = state.pending ? await bounded(state.pending) : {};
    context.mock.restoreAll();
    syncBuiltinESMExports();
    await fs.rm(root, { recursive: true, force: true });
    await assert.rejects(fs.stat(root), { code: 'ENOENT' });
    assert.notEqual(result.timedOut, true, 'Owned startup must settle');
  });
  state.bytes = Buffer.from(JSON.stringify({ version: 1, environment: {}, config: {
    backend: 'http', upstream: 'http://127.0.0.1:9', host: '127.0.0.1', port: 0,
  } }));
  await fs.writeFile(state.path, state.bytes, { mode: 0o600 });
  events.stdout = events.stderr = { write(text) { state.output += text; } };
  state.start = command => {
    state.pending = run([command], {
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
    test(`${command} cancels pending default path access on ${signal}`, { timeout: 5000 }, async context => {
      const state = await fixture(context);
      const originalAccess = fs.access;
      const entered = Promise.withResolvers(), released = Promise.withResolvers();
      state.release = () => released.resolve();
      context.mock.method(fs, 'access', async function (path, ...options) {
        await originalAccess.call(this, path, ...options);
        if (String(path) !== state.path) return;
        entered.resolve({ entered: true });
        await released.promise;
      });
      syncBuiltinESMExports();
      const pending = state.start(command);
      assert.equal((await bounded(Promise.race([entered.promise, pending]))).entered, true);
      assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => state.events.listenerCount(name)), [1, 1]);
      state.events.emit(signal);
      const beforeRelease = await bounded(pending);
      released.resolve();
      const afterRelease = await bounded(pending);
      context.diagnostic(JSON.stringify({ command, signal,
        stoppedBeforeRelease: beforeRelease.value === 0, afterCode: afterRelease.value,
        calls: state.calls, records: state.records }));
      assert.deepEqual(await fs.readFile(state.path), state.bytes);
      assert.deepEqual(state.calls, []);
      assert.deepEqual(state.records, []);
      assert.equal(state.output, '');
      assert.equal(afterRelease.value, 0);
      for (const name of ['SIGINT', 'SIGTERM']) assert.equal(state.events.listenerCount(name), 0);
      assert.equal(beforeRelease.value, 0, 'Startup must stop without waiting for access()');
    });
  }
}
