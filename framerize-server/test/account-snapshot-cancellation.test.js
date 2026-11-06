import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setImmediate as nextTurn } from 'node:timers/promises';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
const { runAccountsCommand, runConfigureCommand } = await import(
  pathToFileURL(join(root, 'dist/account-cli.js')).href);
const add = ['add', 'pending', '--project-id', 'synthetic', '--session-stdin', '--json'];
const session = 'session=synthetic-account';

async function bounded(pending, milliseconds = 1500) {
  let timer;
  try {
    return await Promise.race([pending, new Promise(resolve => {
      timer = setTimeout(() => resolve({ timedOut: true }), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function privateAccounts(context, text = '[]\n') {
  const home = await fs.mkdtemp(join(tmpdir(), 'account-snapshot-cancel-'));
  const path = join(home, 'accounts.json');
  await fs.writeFile(path, text, { mode: 0o600 });
  const state = { home, path, text, cleanup: async () => {} };
  context.after(async () => {
    try { await state.cleanup(); }
    finally {
      await fs.rm(home, { recursive: true, force: true });
      await assert.rejects(fs.stat(home), { code: 'ENOENT' });
    }
  });
  return state;
}

test('cancelled session input starts no snapshot read', { timeout: 5000 }, async context => {
  const state = await privateAccounts(context);
  const controller = new AbortController();
  const reason = new Error('synthetic session cancellation');
  const release = Promise.withResolvers();
  const originalOpen = fs.open;
  let pending, opens = 0, abortedOpens = 0, output = '';
  state.cleanup = async () => {
    release.resolve();
    try { if (pending) assert.notEqual((await bounded(pending)).timedOut, true); }
    finally { context.mock.restoreAll(); syncBuiltinESMExports(); }
  };
  context.mock.method(fs, 'open', async (...args) => {
    if (args[0] === state.path) {
      opens++;
      if (controller.signal.aborted) abortedOpens++;
      if (opens === 2) await release.promise;
    }
    return originalOpen(...args);
  });
  syncBuiltinESMExports();
  pending = runAccountsCommand(add, {
    env: { FRAMERIZE_SERVER_HOME: state.home }, signal: controller.signal,
    stdout: { write(text) { output += text; } },
    readStdin: async () => { controller.abort(reason); return session; },
  }).then(value => ({ value }), error => ({ error }));
  const beforeRelease = await bounded(pending, 400);
  release.resolve();
  assert.equal((await bounded(pending)).error, reason);
  assert.equal(await fs.readFile(state.path, 'utf8'), state.text);
  assert.equal(output, '');
  assert.equal(beforeRelease.error, reason, 'cancel before releasing the snapshot read');
  assert.equal(opens, 1);
  assert.equal(abortedOpens, 0);
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});

for (const phase of ['initial', 'commit', 'configure']) {
  for (const stage of ['open', 'read']) {
    test(`${phase} account ${stage} cancels before release`, { timeout: 6000 }, async context => {
      const state = await privateAccounts(context);
      const controller = new AbortController();
      const reason = new Error(`synthetic ${phase} ${stage} cancellation`);
      const entered = Promise.withResolvers();
      const release = Promise.withResolvers();
      const closed = Promise.withResolvers();
      const originalOpen = fs.open;
      let pending, heldHandle, opens = 0, closes = 0, reads = 0, output = '';
      const pause = async () => { entered.resolve({ entered: true }); await release.promise; };
      state.cleanup = async () => {
        controller.abort(reason);
        release.resolve();
        try {
          if (pending) assert.notEqual((await bounded(pending)).timedOut, true);
          if (heldHandle) {
            if (closes === 0) await heldHandle.close();
            assert.notEqual((await bounded(closed.promise)).timedOut, true);
          }
          await nextTurn();
        } finally { context.mock.restoreAll(); syncBuiltinESMExports(); }
      };
      context.mock.method(fs, 'open', async (...args) => {
        const handle = await originalOpen(...args);
        if (args[0] !== state.path || ++opens !== (phase === 'commit' ? 2 : 1)) return handle;
        heldHandle = handle;
        const close = handle.close.bind(handle);
        const read = handle.read.bind(handle);
        context.mock.method(handle, 'close', async () => {
          closes++;
          try { await close(); } finally { closed.resolve({ closed: true }); }
        });
        context.mock.method(handle, 'read', async (...readArgs) => {
          reads++;
          const value = await read(...readArgs);
          if (stage === 'read' && reads === 1) await pause();
          return value;
        });
        if (stage === 'open') await pause();
        return handle;
      });
      syncBuiltinESMExports();
      const deps = {
        env: { FRAMERIZE_SERVER_HOME: state.home }, signal: controller.signal,
        stdout: { write(text) { output += text; } }, readStdin: async () => session,
      };
      pending = (phase === 'configure'
        ? runConfigureCommand(['--backend', 'framer', '--account-id', 'pending', '--json'], deps)
        : runAccountsCommand(add, deps)).then(value => ({ value }), error => ({ error }));
      assert.equal((await bounded(Promise.race([entered.promise, pending]))).entered, true);
      controller.abort(reason);
      const beforeRelease = await bounded(pending, 400);
      const beforeOutput = output;
      release.resolve();
      assert.equal((await bounded(pending)).error, reason);
      assert.equal((await bounded(closed.promise)).closed, true);
      await nextTurn();
      assert.equal(await fs.readFile(state.path, 'utf8'), state.text);
      assert.deepEqual(await fs.readdir(state.home), ['accounts.json']);
      assert.equal(output, '');
      assert.equal(beforeOutput, '');
      assert.equal(beforeRelease.error, reason, 'cancel before releasing file I/O');
      assert.equal(closes, 1);
      assert.equal(reads, stage === 'open' ? 0 : 1);
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    });
  }
}

for (const withSignal of [false, true]) {
  test(`account read diagnostics remain intact with signal=${withSignal}`, async context => {
    const state = await privateAccounts(context, '{');
    const controller = new AbortController();
    let output = '';
    const deps = { env: { FRAMERIZE_SERVER_HOME: state.home },
      stdout: { write(text) { output += text; } },
      ...(withSignal ? { signal: controller.signal } : {}) };
    await assert.rejects(runAccountsCommand(['list', '--json'], deps),
      { code: 'configuration_error', message: 'Account metadata unavailable' });
    await fs.writeFile(state.path, '[{"id":"valid","unexpected":true}]');
    await assert.rejects(runAccountsCommand(['list', '--json'], deps),
      { code: 'configuration_error', message: 'accounts[0] has an unknown field' });
    assert.equal(output, '');
    await fs.rm(state.path);
    assert.equal(await runAccountsCommand(['list', '--json'], deps), 0);
    assert.deepEqual(JSON.parse(output).accounts, []);
    assert.deepEqual(await fs.readdir(state.home), []);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  });
}
