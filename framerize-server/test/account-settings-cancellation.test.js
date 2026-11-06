import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/account-cli.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
});
stop();
assert.ok(Object.keys(compiled.metafile.inputs).every(path => !/castle|backend\.ts|browser-login/.test(path)));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
console.log(JSON.stringify({ runtime: process.version, bundle: hash(compiled.outputFiles[0].contents),
  inputs: await Promise.all(Object.keys(compiled.metafile.inputs).sort().map(async path =>
    [path, hash(await fs.readFile(path))])) }));
const api = await import(`data:text/javascript;base64,${
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
  const root = await fs.mkdtemp(join(tmpdir(), 'account-settings-cancel-'));
  const controller = new AbortController();
  const state = { root, controller, output: '', pending: undefined, release: () => {} };
  context.after(async () => {
    state.release();
    const result = state.pending ? await bounded(state.pending) : {};
    context.mock.restoreAll();
    syncBuiltinESMExports();
    await fs.rm(root, { recursive: true, force: true });
    await assert.rejects(fs.stat(root), { code: 'ENOENT' });
    assert.notEqual(result.timedOut, true, 'Owned command must settle during cleanup');
  });
  state.settingsPath = join(root, 'server.json');
  state.settings = Buffer.from(JSON.stringify({ version: 1, environment: {}, config: {
    backend: 'http', upstream: 'http://127.0.0.1:9', host: '127.0.0.1', port: 8789,
  } }));
  await fs.writeFile(state.settingsPath, state.settings, { mode: 0o600 });
  await fs.writeFile(join(root, 'accounts.json'), '[]\n', { mode: 0o600 });
  state.invoke = kind => {
    const deps = { signal: controller.signal, env: { HOME: root, FRAMERIZE_SERVER_HOME: root },
      stdout: { write(text) { state.output += text; } } };
    state.pending = (kind === 'accounts' ? api.runAccountsCommand(['list', '--json'], deps)
      : api.runConfigureCommand(['--show', '--json'], deps))
      .then(value => ({ value }), error => ({ error }));
    return state.pending;
  };
  return state;
}

for (const [kind, heldRead] of [['accounts', 1], ['configure', 1], ['configure', 2]]) {
  test(`${kind} cancels while settings read ${heldRead} is pending`, { timeout: 5000 }, async context => {
    const state = await fixture(context);
    const originalOpen = fs.open;
    const originalRead = fs.readFile;
    const entered = Promise.withResolvers();
    const released = Promise.withResolvers();
    const handles = [];
    let reads = 0;
    state.release = () => released.resolve();
    context.mock.method(fs, 'open', async function (path, ...args) {
      const handle = await originalOpen.call(this, path, ...args);
      if (String(path) !== state.settingsPath) return handle;
      const record = { closes: 0, closed: Promise.withResolvers() };
      handles.push(record);
      const read = handle.readFile.bind(handle), close = handle.close.bind(handle);
      context.mock.method(handle, 'close', async () => {
        record.closes++;
        try { return await close(); } finally { record.closed.resolve(); }
      });
      context.mock.method(handle, 'readFile', async (...options) => {
        const bytes = await read(...options);
        if (++reads === heldRead) {
          entered.resolve({ entered: true });
          await released.promise;
        }
        return bytes;
      });
      return handle;
    });
    syncBuiltinESMExports();
    const pending = state.invoke(kind);
    const started = await bounded(Promise.race([entered.promise, pending]));
    assert.equal(started.entered, true, 'The command must reach the chosen settings read');
    const reason = new Error('synthetic account settings cancellation');
    state.controller.abort(reason);
    const beforeRelease = await bounded(pending);
    const outputBeforeRelease = state.output;
    released.resolve();
    const afterRelease = await bounded(pending);
    const closed = await Promise.all(handles.map(record => bounded(record.closed.promise)));
    const unchanged = (await originalRead(state.settingsPath)).equals(state.settings);
    const listeners = getEventListeners(state.controller.signal, 'abort').length;
    context.diagnostic(JSON.stringify({ kind, heldRead,
      cancelledBeforeRelease: beforeRelease.error === reason,
      cancelledAfterRelease: afterRelease.error === reason,
      outputBeforeRelease: outputBeforeRelease.length, outputAfterRelease: state.output.length,
      unchanged, closes: handles.map(record => record.closes), listeners }));
    assert.ok(closed.every(result => !result?.timedOut), 'Every owned handle must close');
    assert.ok(handles.every(record => record.closes === 1));
    assert.equal(unchanged, true);
    assert.equal(listeners, 0);
    assert.deepEqual(await fs.readdir(state.root), ['accounts.json', 'server.json']);
    assert.equal(beforeRelease.error, reason, 'Cancellation must not wait for the held settings read');
    assert.equal(afterRelease.error, reason, 'Late completion must preserve cancellation');
    assert.equal(state.output, '', 'Cancelled commands must not print successful results');
  });
}

for (const kind of ['accounts', 'configure']) {
  test(`${kind} rejects a pre-aborted request before opening settings`, async context => {
    const state = await fixture(context);
    const originalOpen = fs.open;
    let opens = 0;
    context.mock.method(fs, 'open', async (...args) => {
      opens++;
      return originalOpen(...args);
    });
    syncBuiltinESMExports();
    const reason = new Error('synthetic pre-aborted account command');
    state.controller.abort(reason);
    const result = await state.invoke(kind);
    context.diagnostic(JSON.stringify({ kind, preAborted: true, opens,
      cancelled: result.error === reason, outputLength: state.output.length }));
    assert.equal(result.error, reason);
    assert.equal(opens, 0);
    assert.equal(state.output, '');
  });

  test(`${kind} preserves uncancelled settings output`, async context => {
    const state = await fixture(context);
    assert.equal((await state.invoke(kind)).value, 0);
    const output = JSON.parse(state.output);
    if (kind === 'accounts') assert.deepEqual(output.accounts, []);
    else {
      assert.equal(output.config.backend, 'http');
      assert.equal(output.config.upstream, 'http://127.0.0.1:9/');
      assert.equal(output.sources.upstream, 'saved');
    }
    assert.deepEqual(await fs.readFile(state.settingsPath), state.settings);
    assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
  });
}
