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
const { loadAccounts, loadFramerAccount, readAccountText, ACCOUNT_FILE_MAX_BYTES } = await import(
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
      timer = setTimeout(() => reject(new Error('Fixture did not settle')), 1500);
    })]);
  } finally { clearTimeout(timer); }
}

for (const stage of ['open', 'stat', 'close']) {
  test(`metadata cancellation releases a pending ${stage}`, { timeout: 6000 }, async context => {
    const root = await fs.mkdtemp(join(tmpdir(), 'account-read-control-'));
    const path = join(root, 'accounts.json');
    const held = deferred();
    const release = deferred();
    const closed = deferred();
    const controller = new AbortController();
    const reason = new Error('synthetic cancellation');
    const originalOpen = fs.open;
    let handle;
    let operation;
    let closeCalls = 0;
    let statCalls = 0;
    context.after(async () => {
      controller.abort(reason);
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
    await fs.writeFile(path, '[]', { mode: 0o600 });
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
        const info = await stat();
        if (stage === 'stat') await pause();
        return info;
      });
      if (stage === 'open') await pause();
      return opened;
    });
    syncBuiltinESMExports();
    operation = readAccountText(path, false, controller.signal).then(
      value => ({ value }), error => ({ error }));
    await within(Promise.race([held.promise, operation.then(() => {
      throw new Error('Read settled before the fixture held its operation');
    })]));
    controller.abort(reason);
    const result = await within(operation);
    assert.equal(result.error, reason);
    assert.equal(closeCalls, stage === 'open' ? 0 : 1);
    release.resolve();
    await within(closed.promise);
    await nextTurn();
    assert.equal(closeCalls, 1);
    assert.equal(statCalls, stage === 'open' ? 0 : 1);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    assert.equal(await fs.readFile(path, 'utf8'), '[]');
    context.diagnostic(`${stage}: cancelled before release; one close; no remaining abort listener`);
  });
}

test('pre-aborted metadata reads perform no I/O', async context => {
  let calls = 0;
  const reason = new Error('synthetic pre-abort');
  const signal = AbortSignal.abort(reason);
  context.mock.method(fs, 'open', () => { calls++; throw new Error('Unexpected file open'); });
  syncBuiltinESMExports();
  context.after(() => { context.mock.restoreAll(); syncBuiltinESMExports(); });
  await assert.rejects(readAccountText('/not-opened', false, signal), error => error === reason);
  await assert.rejects(loadAccounts('/not-opened', signal), error => error === reason);
  await assert.rejects(loadFramerAccount('/not-opened', undefined, signal), error => error === reason);
  assert.equal(calls, 0);
  assert.equal(getEventListeners(signal, 'abort').length, 0);
});

for (const withSignal of [false, true]) {
  test(`metadata results stay unchanged with signal=${withSignal}`, async context => {
    const root = await fs.mkdtemp(join(tmpdir(), 'account-read-results-'));
    context.after(() => fs.rm(root, { recursive: true, force: true }));
    const path = join(root, 'accounts.json');
    const missing = join(root, 'missing.json');
    const signal = withSignal ? new AbortController().signal : undefined;
    const account = { id: 'synthetic', projectId: 'synthetic-project',
      session: { env: 'SYNTHETIC_SESSION' } };
    await fs.writeFile(path, JSON.stringify([account]), { mode: 0o600 });
    const loaded = await loadAccounts(path, signal);
    assert.deepEqual(loaded, [account]);
    assert(Object.isFrozen(loaded));
    assert(Object.isFrozen(loaded[0]));
    assert.deepEqual(await loadFramerAccount(path, 'synthetic', signal), account);
    assert.deepEqual(await loadAccounts(undefined, signal), []);
    assert.equal(await readAccountText(missing, true, signal), undefined);
    await assert.rejects(readAccountText(missing, false, signal),
      { code: 'configuration_error', message: 'Account metadata unavailable' });
    await fs.writeFile(path, '{');
    await assert.rejects(loadAccounts(path, signal),
      { code: 'configuration_error', message: 'Account metadata unavailable' });
    await fs.writeFile(path, Buffer.alloc(ACCOUNT_FILE_MAX_BYTES + 1, 32));
    await assert.rejects(readAccountText(path, false, signal),
      { code: 'configuration_error', message: 'Account metadata unavailable' });
    if (signal) assert.equal(getEventListeners(signal, 'abort').length, 0);
  });
}
