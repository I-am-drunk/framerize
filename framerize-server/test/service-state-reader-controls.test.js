import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs/promises';
import { constants } from 'node:fs';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export * from './service-control.ts'; export { selectServiceManager } from './service-units.ts';",
  resolveDir: fileURLToPath(new URL('../src/', import.meta.url)), loader: 'ts',
}, bundle: true, platform: 'node', format: 'esm', write: false });
stop();
const { serviceStatus, controlService, reconcileService, selectServiceManager, ServerSetupError } =
  await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const planFor = root => ({ platform: 'linux', uid: 7342, home: join(root, 'home'),
  homeDir: root, xdgConfigHome: join(root, 'config'),
  program: [process.execPath, join(root, 'unused.js')], environmentPath: '/synthetic/bin' });

async function within(pending, phase) {
  let timer;
  try { return await Promise.race([pending, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`Timed out during ${phase}`)), 1500);
  })]); } finally { clearTimeout(timer); }
}

const forbidden = async () => { throw new Error('Unexpected service command'); };

for (const [phase, fails] of [
  ['open', false], ['stat', false], ['readFile', false], ['close', false],
  ['stat', true], ['readFile', true], ['close', true],
]) {
  test(`service status cancels during ${phase}, late ${fails ? 'failure' : 'success'}`,
    { timeout: 6000 }, async context => {
      const root = await fs.mkdtemp(join(tmpdir(), 'server-state-control-'));
      const plan = planFor(root), manager = selectServiceManager(plan);
      const controller = new AbortController(), reason = new Error('Synthetic cancellation');
      const entered = Promise.withResolvers(), gate = Promise.withResolvers();
      const closed = Promise.withResolvers(), operations = [];
      let handle, originalClose, pending, cleaned = false;
      let closes = 0, stats = 0, reads = 0, flags;
      const cleanup = async () => {
        if (cleaned) return;
        controller.abort(reason);
        gate.resolve();
        if (pending) await within(pending, 'cleanup settlement');
        await within(Promise.allSettled(operations), 'operation cleanup');
        if (handle?.fd >= 0) await originalClose();
        context.mock.restoreAll();
        syncBuiltinESMExports();
        await fs.rm(root, { recursive: true, force: true });
        cleaned = true;
      };
      context.after(cleanup);
      await fs.mkdir(dirname(manager.path), { recursive: true });
      await fs.writeFile(manager.path, manager.contents, { mode: 0o600 });
      const hold = (stage, operation) => {
        const work = Promise.resolve(operation).then(async value => {
          if (stage === phase) {
            entered.resolve();
            await gate.promise;
            if (fails) throw new Error('Synthetic late operation failure');
          }
          return value;
        });
        operations.push(work);
        return work;
      };
      const originalOpen = fs.open.bind(fs);
      context.mock.method(fs, 'open', async (path, suppliedFlags) => {
        assert.equal(path, manager.path);
        flags = suppliedFlags;
        handle = await originalOpen(path, suppliedFlags);
        const stat = handle.stat.bind(handle), read = handle.readFile.bind(handle);
        originalClose = handle.close.bind(handle);
        context.mock.method(handle, 'stat', () => { stats++; return hold('stat', stat()); });
        context.mock.method(handle, 'readFile', options => {
          reads++;
          assert.equal(options.signal, controller.signal);
          return hold('readFile', read(options));
        });
        context.mock.method(handle, 'close', () => {
          closes++;
          return hold('close', originalClose().then(() => closed.resolve()));
        });
        return hold('open', handle);
      });
      syncBuiltinESMExports();
      pending = serviceStatus(plan, forbidden, controller.signal).then(
        value => ({ value }), error => ({ error }));
      await within(entered.promise, `${phase} entry`);
      controller.abort(reason);
      assert.equal((await within(pending, 'cancellation before release')).error, reason);
      if (phase !== 'open') {
        await within(closed.promise, 'close before release');
        assert.equal(handle.fd, -1);
      }
      gate.resolve();
      await within(closed.promise, 'eventual file close');
      await within(Promise.allSettled(operations), 'late operations');
      assert.equal(closes, 1);
      assert.equal(handle.fd, -1);
      assert.equal(stats, phase === 'open' ? 0 : 1);
      assert.equal(reads, ['open', 'stat'].includes(phase) ? 0 : 1);
      assert.equal(flags, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOCTTY);
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
      assert.equal(await fs.readFile(manager.path, 'utf8'), manager.contents);
      await cleanup();
      await assert.rejects(fs.access(root), { code: 'ENOENT' });
    });
}

test('pre-aborted service operations perform no file or command work', async context => {
  const controller = new AbortController(), reason = new Error('Already cancelled');
  controller.abort(reason);
  let opens = 0, commands = 0;
  context.mock.method(fs, 'open', async () => { opens++; throw new Error('Unexpected open'); });
  syncBuiltinESMExports();
  context.after(() => { context.mock.restoreAll(); syncBuiltinESMExports(); });
  const plan = planFor(join(tmpdir(), 'unused-service-state'));
  const run = async () => { commands++; throw new Error('Unexpected command'); };
  const actions = [() => serviceStatus(plan, run, controller.signal),
    ...['start', 'stop', 'restart', 'uninstall'].map(action =>
      () => controlService(plan, action, run, controller.signal)),
    () => reconcileService(plan, '{}', forbidden, run, forbidden, controller.signal)];
  for (const action of actions) await assert.rejects(action, error => error === reason);
  assert.equal(opens, 0);
  assert.equal(commands, 0);
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});

for (const mode of ['nonregular', 'stat-error', 'read-error']) {
  test(`service status closes rejected state after ${mode}`, async context => {
    const root = await fs.mkdtemp(join(tmpdir(), 'server-state-rejection-'));
    const plan = planFor(root), manager = selectServiceManager(plan);
    const controller = new AbortController(), failure = new Error('Synthetic I/O failure');
    let handle, closes = 0, reads = 0;
    context.after(async () => {
      context.mock.restoreAll();
      syncBuiltinESMExports();
      if (handle?.fd >= 0) await handle.close();
      await fs.rm(root, { recursive: true, force: true });
    });
    await fs.mkdir(dirname(manager.path), { recursive: true });
    await fs.writeFile(manager.path, manager.contents, { mode: 0o600 });
    const originalOpen = fs.open.bind(fs);
    context.mock.method(fs, 'open', async (...args) => {
      handle = await originalOpen(...args);
      const stat = handle.stat.bind(handle), close = handle.close.bind(handle);
      context.mock.method(handle, 'stat', async () => {
        if (mode === 'stat-error') throw failure;
        return mode === 'nonregular' ? { isFile: () => false } : stat();
      });
      context.mock.method(handle, 'readFile', async () => { reads++; throw failure; });
      context.mock.method(handle, 'close', () => { closes++; return close(); });
      return handle;
    });
    syncBuiltinESMExports();
    await assert.rejects(serviceStatus(plan, forbidden, controller.signal), error =>
      mode === 'nonregular' ? error instanceof ServerSetupError
        && error.message === 'Service state must be a regular file.' : error === failure);
    assert.equal(closes, 1);
    assert.equal(handle.fd, -1);
    assert.equal(reads, mode === 'read-error' ? 1 : 0);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    assert.equal(await fs.readFile(manager.path, 'utf8'), manager.contents);
  });
}
