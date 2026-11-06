import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import https from 'node:https';
import net from 'node:net';
import childProcess from 'node:child_process';
import { createHash } from 'node:crypto';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const core = fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url));
const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/setup.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
  plugins: [{ name: 'source-boundaries', setup(builder) {
    builder.onResolve({ filter: /^\.\/(backend|account-cli|browser-login)\.js$/ },
      args => ({ path: args.path, external: true }));
    builder.onResolve({ filter: /\/dist\/core\.js$/ }, () => ({ path: core }));
  } }],
});
stop();
assert.ok(Object.keys(compiled.metafile.inputs).every(path => !/castle|backend\.ts|browser-login/.test(path)));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
console.log(JSON.stringify({ runtime: process.version, bundle: hash(compiled.outputFiles[0].contents),
  inputs: await Promise.all(Object.keys(compiled.metafile.inputs).sort().map(async path =>
    [path, hash(await fs.readFile(path))])) }));
const { runSetupCommand } = await import(`data:text/javascript;base64,${
  Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function bounded(pending, milliseconds = 1000) {
  let timer;
  try {
    return await Promise.race([pending, new Promise(resolve => {
      timer = setTimeout(() => resolve({ timedOut: true }), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function fixture(context, command) {
  const root = await fs.mkdtemp(join(tmpdir(), 'setup-account-cancel-'));
  const state = { root, controller: new AbortController(), handles: [], records: [],
    pending: undefined, output: '', commands: [], readiness: 0, forbidden: 0, release() {} };
  context.after(async () => {
    state.release();
    state.controller.abort(new Error('fixture cleanup'));
    const settled = await bounded(state.pending ?? Promise.resolve({}));
    for (const handle of state.handles) if (handle.fd !== -1) await handle.close();
    context.mock.restoreAll();
    syncBuiltinESMExports();
    await fs.rm(root, { recursive: true, force: true });
    await assert.rejects(fs.stat(root), { code: 'ENOENT' });
    assert.notEqual(settled.timedOut, true, 'Owned setup must settle during cleanup');
    context.diagnostic('owned handles closed; private state removed');
  });
  state.path = join(root, 'accounts.json');
  state.bytes = Buffer.from('[]\n');
  await fs.writeFile(state.path, state.bytes, { mode: 0o600 });
  state.originalRead = fs.readFile.bind(fs);
  const deny = () => { state.forbidden++; throw new Error('Unexpected external operation'); };
  context.mock.method(globalThis, 'fetch', deny);
  for (const target of [http, https]) {
    for (const name of ['request', 'get', 'createServer']) context.mock.method(target, name, deny);
  }
  for (const name of ['connect', 'createConnection', 'createServer']) context.mock.method(net, name, deny);
  for (const name of ['spawn', 'exec', 'execFile', 'spawnSync', 'execSync', 'execFileSync', 'fork']) {
    context.mock.method(childProcess, name, deny);
  }
  syncBuiltinESMExports();
  state.invoke = () => {
    state.pending = runSetupCommand([...command, '--backend', 'http', '--upstream',
      'http://127.0.0.1:9', '--accounts-file', state.path, '--host', '127.0.0.1',
      '--port', '8789', '--no-lan', '--json'], {
      signal: state.controller.signal, platform: 'linux', uid: 501,
      env: { HOME: root, XDG_CONFIG_HOME: join(root, 'xdg'), FRAMERIZE_SERVER_HOME: root },
      program: [process.execPath, join(root, 'synthetic-server.js')],
      stdout: { write(text) { state.output += text; } },
      run: async (program, args) => {
        state.commands.push([program, ...args]);
        throw new Error('Unexpected service command');
      },
      ready: async () => { state.readiness++; throw new Error('Unexpected readiness'); },
    }).then(value => ({ value }), error => ({ error }));
    return state.pending;
  };
  return state;
}

async function unchanged(state) {
  assert.deepEqual(await state.originalRead(state.path), state.bytes);
  assert.deepEqual(await fs.readdir(state.root), ['accounts.json']);
  assert.equal(state.output, '');
  assert.deepEqual(state.commands, []);
  assert.equal(state.readiness, 0);
  assert.equal(state.forbidden, 0);
}

function holdAccounts(context, state, phase) {
  const entered = Promise.withResolvers();
  const released = Promise.withResolvers();
  const open = fs.open;
  state.release = () => released.resolve();
  context.mock.method(fs, 'open', async function (path, ...options) {
    const handle = await open.call(this, path, ...options);
    if (String(path) !== state.path) return handle;
    state.handles.push(handle);
    const record = { closes: 0, closed: Promise.withResolvers() };
    state.records.push(record);
    const read = handle.read.bind(handle), close = handle.close.bind(handle);
    context.mock.method(handle, 'close', async () => {
      record.closes++;
      try { return await close(); } finally { record.closed.resolve({ closed: true }); }
    });
    const pause = async () => {
      entered.resolve({ entered: true });
      await released.promise;
    };
    if (phase === 'open') await pause();
    else context.mock.method(handle, 'read', async (...args) => {
      const result = await read(...args);
      if (result.bytesRead) await pause();
      return result;
    });
    return handle;
  });
  syncBuiltinESMExports();
  return entered.promise;
}

for (const command of [['up'], ['service', 'install']]) {
  for (const phase of ['open', 'read']) {
    test(`${command.join(' ')} cancels during account ${phase}`, { timeout: 5000 }, async context => {
      const state = await fixture(context, command);
      const entered = holdAccounts(context, state, phase);
      const pending = state.invoke();
      assert.equal((await bounded(Promise.race([entered, pending]))).entered, true,
        'Setup must reach the held account operation');
      const reason = new Error('synthetic setup account cancellation');
      state.controller.abort(reason);
      const beforeRelease = await bounded(pending);
      await unchanged(state);
      const closedBeforeRelease = state.handles[0].fd === -1;
      state.release();
      const settled = await bounded(pending);
      assert.equal(settled.error, reason, 'Preserve the caller cancellation reason');
      assert.equal((await bounded(state.records[0].closed.promise)).closed, true);
      assert.equal(state.records[0].closes, 1);
      assert.equal(state.handles[0].fd, -1);
      assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
      await unchanged(state);
      context.diagnostic(JSON.stringify({ phase, settledBeforeRelease: beforeRelease.error === reason,
        closedBeforeRelease, commands: state.commands.length, readiness: state.readiness,
        forbidden: state.forbidden, closes: state.records[0].closes }));
      assert.equal(beforeRelease.error, reason, 'Setup must reject before account I/O is released');
    });
  }
}

test('pre-aborted up does not open account state', { timeout: 5000 }, async context => {
  const state = await fixture(context, ['up']);
  let opens = 0;
  context.mock.method(fs, 'open', async () => { opens++; throw new Error('Unexpected open'); });
  syncBuiltinESMExports();
  const reason = new Error('cancel before setup');
  state.controller.abort(reason);
  assert.equal((await bounded(state.invoke())).error, reason);
  assert.equal(opens, 0);
  assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
  await unchanged(state);
});
