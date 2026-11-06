import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import https from 'node:https';
import childProcess from 'node:child_process';
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
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
console.log(JSON.stringify({ runtime: process.version, bundle: digest(compiled.outputFiles[0].contents),
  inputs: await Promise.all(Object.keys(compiled.metafile.inputs).sort().map(async path =>
    [path, digest(await fs.readFile(path))])) }));
const { run } = await import(`data:text/javascript;base64,${
  Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function bounded(pending, milliseconds = 1000) {
  let timer;
  try {
    return await Promise.race([pending, new Promise(resolve => {
      timer = setTimeout(() => resolve({ timedOut: true }), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function fixture(context) {
  const root = await fs.mkdtemp(join(tmpdir(), 'server-account-startup-'));
  const state = { root, events: new EventEmitter(), records: [], servers: [], handles: [],
    release() {}, pending: undefined, output: '', providerCloses: 0, forbidden: 0 };
  context.after(async () => {
    state.release();
    state.events.emit('SIGTERM');
    const result = await bounded(state.pending ?? Promise.resolve({}));
    for (const handle of state.handles) await handle.close();
    for (const server of state.servers) await new Promise(resolve => server.close(resolve));
    context.mock.restoreAll();
    syncBuiltinESMExports();
    await fs.rm(root, { recursive: true, force: true });
    await assert.rejects(fs.stat(root), { code: 'ENOENT' });
    assert.notEqual(result.timedOut, true, 'owned startup must finish during cleanup');
    context.diagnostic('owned file handles and servers closed; private state removed');
  });
  state.path = join(root, 'accounts.json');
  state.bytes = Buffer.from('[]\n');
  await fs.writeFile(state.path, state.bytes, { mode: 0o600 });
  state.events.stdout = state.events.stderr = { write(text) { state.output += text; } };
  return state;
}

function start(context, state, command) {
  const originalCreateServer = http.createServer;
  context.mock.method(http, 'createServer', (...args) => {
    const server = originalCreateServer(...args);
    state.servers.push(server);
    return server;
  });
  const forbidden = () => { state.forbidden++; throw new Error('external work is forbidden'); };
  context.mock.method(http, 'request', forbidden);
  context.mock.method(https, 'request', forbidden);
  context.mock.method(globalThis, 'fetch', forbidden);
  for (const name of ['spawn', 'execFile', 'exec']) context.mock.method(childProcess, name, forbidden);
  syncBuiltinESMExports();
  state.pending = run([command, '--backend', 'http', '--upstream', 'http://127.0.0.1:9',
    '--accounts-file', state.path, '--host', '127.0.0.1', '--port', '0'], {
    process: state.events, env: { HOME: state.root, FRAMERIZE_SERVER_HOME: state.root },
    logger: { info(event) {
      state.records.push(event);
      if (event === 'server.listening') state.events.emit('SIGTERM');
    } },
    createProvider() { return { close() { state.providerCloses++; } }; },
  }).then(value => ({ value }), error => ({ error }));
  return state.pending;
}

for (const command of ['check', 'serve']) {
  for (const stage of ['open', 'read']) {
    const signal = stage === 'open' ? 'SIGINT' : 'SIGTERM';
    test(`${command} cancels account ${stage} during server construction`, { timeout: 6000 }, async context => {
      const state = await fixture(context);
      const entered = Promise.withResolvers(), released = Promise.withResolvers();
      const fileClosed = Promise.withResolvers();
      const originalOpen = fs.open;
      let holds = 0, closes = 0;
      state.release = () => released.resolve();
      context.mock.method(fs, 'open', async (...args) => {
        const handle = await originalOpen(...args);
        if (String(args[0]) !== state.path) return handle;
        state.handles.push(handle);
        const close = handle.close.bind(handle);
        context.mock.method(handle, 'close', async () => {
          closes++;
          try { await close(); } finally { fileClosed.resolve(); }
        });
        const hold = async () => {
          holds++;
          entered.resolve({ entered: true });
          await released.promise;
        };
        if (stage === 'open') await hold();
        else {
          const read = handle.read.bind(handle);
          context.mock.method(handle, 'read', async (...values) => {
            const result = await read(...values);
            if (holds === 0) await hold();
            return result;
          });
        }
        return handle;
      });
      const pending = start(context, state, command);
      assert.equal((await bounded(Promise.race([entered.promise, pending]))).entered, true);
      state.events.emit(signal);
      const beforeRelease = await bounded(pending);
      const closedBeforeRelease = closes;
      released.resolve();
      const afterRelease = await bounded(pending);
      assert.notEqual((await bounded(fileClosed.promise))?.timedOut, true);
      context.diagnostic(JSON.stringify({ command, stage, signal,
        stoppedBeforeRelease: beforeRelease.value === 0, closedBeforeRelease,
        afterCode: afterRelease.value, serversCreated: state.servers.length,
        providerCloses: state.providerCloses, holds, closes, forbidden: state.forbidden }));
      assert.equal(afterRelease.value, 0);
      assert.equal(state.providerCloses, 1);
      assert.equal(holds, 1);
      assert.equal(closes, 1);
      assert.equal(state.output, '');
      assert.deepEqual(state.records, []);
      assert.equal(state.forbidden, 0);
      assert.deepEqual(await fs.readFile(state.path), state.bytes);
      for (const name of ['SIGINT', 'SIGTERM']) assert.equal(state.events.listenerCount(name), 0);
      assert.equal(beforeRelease.value, 0, 'startup must stop before the account operation is released');
      assert.equal(closedBeforeRelease, stage === 'read' ? 1 : 0);
      assert.equal(state.servers.length, 0, 'cancelled construction must not create an HTTP server');
    });
  }
}

test('uncancelled check validates the same account file and closes its provider', { timeout: 5000 }, async context => {
  const state = await fixture(context);
  assert.equal((await bounded(start(context, state, 'check'))).value, 0);
  assert.equal(state.providerCloses, 1);
  assert.equal(state.servers.length, 1);
  assert.equal(state.servers[0].listening, false);
  assert.deepEqual(state.records, ['config.valid']);
  assert.equal(state.forbidden, 0);
  assert.deepEqual(await fs.readFile(state.path), state.bytes);
  for (const name of ['SIGINT', 'SIGTERM']) assert.equal(state.events.listenerCount(name), 0);
});
