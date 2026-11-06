import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setImmediate as nextTurn } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/accounts.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { readAccountSession } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

async function within(pending) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error('Fixture wait did not settle')), 1500);
    })]);
  } finally { clearTimeout(timer); }
}

for (const stage of ['open', 'stat', 'close']) {
  test(`session cancellation handles pending ${stage}`, { timeout: 6000 }, async context => {
    const root = await fs.mkdtemp(join(tmpdir(), 'session-cancel-control-'));
    const path = join(root, 'session.txt');
    const held = deferred();
    const release = deferred();
    const closed = deferred();
    const controller = new AbortController();
    const originalOpen = fs.open;
    let handle;
    let operation;
    let closeCalls = 0;
    let statCalls = 0;
    context.after(async () => {
      controller.abort();
      release.resolve();
      try {
        if (operation) await within(operation);
        if (handle) await within(closed.promise);
        await nextTurn();
      } finally {
        if (handle && closeCalls === 0) await handle.close();
        context.mock.restoreAll();
        syncBuiltinESMExports();
        await fs.rm(root, { recursive: true, force: true });
      }
      await assert.rejects(fs.stat(root), { code: 'ENOENT' });
    });
    await fs.writeFile(path, 'synthetic-session\n', { mode: 0o600 });
    context.mock.method(fs, 'open', async (...args) => {
      const opened = await originalOpen(...args);
      if (args[0] !== path) return opened;
      handle = opened;
      const close = handle.close.bind(handle);
      const stat = handle.stat.bind(handle);
      const pause = async () => { held.resolve(); await release.promise; };
      context.mock.method(handle, 'close', async () => {
        closeCalls++;
        await close();
        if (stage === 'close') await pause();
        closed.resolve();
      });
      context.mock.method(handle, 'stat', async () => {
        statCalls++;
        const result = await stat();
        if (stage === 'stat') await pause();
        return result;
      });
      if (stage === 'open') await pause();
      return opened;
    });
    syncBuiltinESMExports();
    operation = readAccountSession({ file: path }, {}, controller.signal)
      .then(value => ({ value }), error => ({ error }));
    await within(Promise.race([held.promise, operation.then(() => {
      throw new Error('Session read settled before its operation was held');
    })]));
    controller.abort(new Error('synthetic cancellation'));
    const result = await within(operation);
    assert.equal(result.error.code, 'configuration_error');
    assert.equal(result.error.message, 'Account session unavailable');
    assert.equal(closeCalls, stage === 'open' ? 0 : 1);
    release.resolve();
    await within(closed.promise);
    await nextTurn();
    assert.equal(closeCalls, 1);
    assert.equal(statCalls, stage === 'open' ? 0 : 1);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    assert.equal(await fs.readFile(path, 'utf8'), 'synthetic-session\n');
    context.diagnostic(`${stage}: cancelled before release; one close; no remaining abort listener`);
  });
}

test('pre-aborted session reads open no file and preserve the public error', async context => {
  let calls = 0;
  context.mock.method(fs, 'open', () => { calls++; throw new Error('Unexpected open'); });
  syncBuiltinESMExports();
  context.after(() => { context.mock.restoreAll(); syncBuiltinESMExports(); });
  await assert.rejects(readAccountSession({ file: '/not-opened' }, {}, AbortSignal.abort()),
    { code: 'configuration_error', message: 'Account session unavailable' });
  assert.equal(calls, 0);
});

for (const withSignal of [false, true]) {
  test(`session validation remains unchanged with signal=${withSignal}`, async context => {
    const root = await fs.mkdtemp(join(tmpdir(), 'session-validation-'));
    context.after(() => fs.rm(root, { recursive: true, force: true }));
    const path = join(root, 'session.txt');
    const signal = withSignal ? new AbortController().signal : undefined;
    const env = { SYNTHETIC_SESSION: 'synthetic-env-session' };
    assert.equal(await readAccountSession('synthetic-inline-session', env, signal),
      'synthetic-inline-session');
    assert.equal(await readAccountSession({ env: 'SYNTHETIC_SESSION' }, env, signal),
      'synthetic-env-session');
    await fs.writeFile(path, 'synthetic-file-session\r\n', { mode: 0o600 });
    assert.equal(await readAccountSession({ file: path }, env, signal), 'synthetic-file-session');
    const invalid = { code: 'configuration_error', message: 'Account session unavailable' };
    for (const bytes of [Buffer.from([0xff]), Buffer.alloc(16_385, 97), Buffer.alloc(0)]) {
      await fs.writeFile(path, bytes);
      await assert.rejects(readAccountSession({ file: path }, env, signal), invalid);
    }
    await assert.rejects(readAccountSession({ file: join(root, 'missing') }, env, signal), invalid);
    if (process.platform !== 'win32') {
      await fs.writeFile(path, 'synthetic-session');
      await fs.chmod(path, 0o644);
      await assert.rejects(readAccountSession({ file: path }, env, signal), invalid);
    }
    if (signal) assert.equal(getEventListeners(signal, 'abort').length, 0);
  });
}
