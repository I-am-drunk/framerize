import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const digest = value => createHash('sha256').update(value).digest('hex');
const compiled = await build({ absWorkingDir: root,
  entryPoints: ['framerize-remote/src/serviceControl.ts'],
  alias: { '@framerize/core': join(root, 'framerize-core/src/index.ts') },
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
stop();
const bytes = compiled.outputFiles[0].contents;
const api = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
console.log(JSON.stringify({ runtime: process.version, bundle: digest(bytes),
  inputs: Object.fromEntries(await Promise.all(Object.keys(compiled.metafile.inputs)
    .map(async path => [path, digest(await fs.readFile(join(root, path)))]))) }));

async function bounded(promise, ms = 800) {
  let timer;
  try {
    return await Promise.race([promise, new Promise(resolve => {
      timer = setTimeout(() => resolve({ timedOut: true }), ms);
    })]);
  } finally { clearTimeout(timer); }
}

async function fixture(context) {
  const home = await fs.mkdtemp(join(tmpdir(), 'frm-unit-read-case-'));
  const controller = new AbortController();
  const path = join(home, '.config/systemd/user/frm.service');
  const unit = `Environment=FRM_HOME=${home}\n`;
  const state = { home, path, unit, controller, calls: [], release() {}, pending: undefined };
  context.after(async () => {
    controller.abort();
    state.release();
    const result = state.pending ? await bounded(state.pending, 2000) : {};
    context.mock.restoreAll();
    syncBuiltinESMExports();
    await fs.rm(home, { recursive: true, force: true });
    await assert.rejects(fs.stat(home), { code: 'ENOENT' });
    assert.notEqual(result.timedOut, true, 'Owned operation must settle during cleanup');
  });
  await fs.mkdir(dirname(path), { recursive: true });
  await fs.writeFile(path, unit, { mode: 0o600 });
  state.service = { host: { platform: 'linux', homeDir: home, uid: 1000 },
    home, program: ['/synthetic/frm', 'serve'], environmentPath: '/synthetic/bin',
    signal: controller.signal, run: async (command, args) => {
      state.calls.push({ command, args });
      return { code: 0, stdout: '' };
    } };
  return state;
}
const operations = {
  status: state => api.serviceStatus(state.service),
  stop: state => api.controlService(state.service, 'stop'),
  uninstall: state => api.uninstallService(state.service),
};
for (const [name, invoke] of Object.entries(operations)) {
  test(`${name} cancels while the unit read is pending`, { timeout: 5000 }, async context => {
    const state = await fixture(context);
    const original = fs.readFile;
    const entered = Promise.withResolvers(), release = Promise.withResolvers();
    state.release = () => release.resolve();
    context.mock.method(fs, 'readFile', async function (path, ...args) {
      const value = await original.call(this, path, ...args);
      if (String(path) === state.path) {
        entered.resolve({ entered: true });
        await release.promise;
      }
      return value;
    });
    syncBuiltinESMExports();
    state.pending = invoke(state).then(value => ({ value }), error => ({ error }));
    assert.equal((await bounded(Promise.race([entered.promise, state.pending]))).entered, true);
    state.controller.abort(new Error('Synthetic unit-read cancellation'));
    const before = await bounded(state.pending);
    release.resolve();
    const after = await bounded(state.pending);
    assert.equal(await original(state.path, 'utf8'), state.unit);
    assert.deepEqual(state.calls, []);
    assert.equal(after.error?.code, 'aborted');
    assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
    context.diagnostic(JSON.stringify({ operation: name, pendingBeforeRelease: before.timedOut === true,
      afterCode: after.error?.code, serviceCalls: state.calls.length, unitUnchanged: true }));
    assert.equal(before.error?.code, 'aborted', 'Cancellation must settle before the unit read is released');
  });
}
test('pre-aborted status starts no unit read', async context => {
  const state = await fixture(context);
  const original = fs.readFile;
  let reads = 0;
  context.mock.method(fs, 'readFile', async (...args) => {
    if (String(args[0]) === state.path) reads++;
    return original(...args);
  });
  syncBuiltinESMExports();
  state.controller.abort();
  state.pending = api.serviceStatus(state.service).then(value => ({ value }), error => ({ error }));
  assert.equal((await state.pending).error?.code, 'aborted');
  assert.deepEqual(state.calls, []);
  context.diagnostic(JSON.stringify({ preAborted: true, reads }));
  assert.equal(reads, 0, 'A cancelled status request must not read its unit');
});
test('ordinary status retains the unit identity and probes', async context => {
  const state = await fixture(context);
  state.pending = api.serviceStatus(state.service).then(value => ({ value }), error => ({ error }));
  const result = await bounded(state.pending);
  assert.equal(result.value.installed, true);
  assert.equal(result.value.otherHome, false);
  assert.equal(result.value.running, true);
  assert.equal(result.value.enabled, true);
  assert.equal(state.calls.length, 2);
  assert.equal(await fs.readFile(state.path, 'utf8'), state.unit);
  assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
});
