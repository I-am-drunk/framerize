import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs/promises';
import http from 'node:http';
import https from 'node:https';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
const root = fileURLToPath(new URL('../../', import.meta.url));
const bundled = await build({ stdin: { contents: `
export { loadRunConfig } from './framerize-remote/src/configStore.ts';
export { setUp } from './framerize-remote/src/cmdUp.ts';
export { clientDependencies } from './framerize-remote/src/clientCli.ts';
export { resolveInstance } from './framerize-remote/src/resolveInstance.ts';
export { installService } from './framerize-remote/src/serviceControl.ts';
`, resolveDir: root }, bundle: true, write: false, platform: 'node', format: 'esm',
  alias: { '@framerize/core': join(root, 'framerize-core/src/index.ts') } });
const { loadRunConfig, setUp, clientDependencies, resolveInstance, installService } = await import(
  `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].contents).toString('base64')}`);

async function bounded(pending) {
  let timer;
  try { return await Promise.race([pending, new Promise((unused, reject) => {
    timer = setTimeout(() => reject(new Error('Owned operation exceeded one second')), 1000);
  })]); } finally { clearTimeout(timer); }
}

async function fixture(context, sample) {
  const home = await fs.mkdtemp(join(tmpdir(), 'frm-profile-cancellation-'));
  const profile = join(home, 'framerize-remote.json');
  const before = '{"remote":"http://127.0.0.1:1","port":8788}\n';
  const controller = new AbortController(), handles = new Set();
  const state = { reads: 0, commands: 0, requests: 0, held: false, pending: undefined };
  let enter, release, closeHeld;
  const entered = new Promise(resolve => { enter = resolve; });
  const gate = new Promise(resolve => { release = resolve; });
  const heldClosed = new Promise(resolve => { closeHeld = resolve; });
  const originalOpen = fs.open;
  context.after(async () => {
    release(); controller.abort();
    try {
      if (state.pending) await bounded(state.pending);
      if (state.held) await bounded(heldClosed);
      await Promise.all([...handles].map(handle => handle.close()));
    } finally {
      context.mock.restoreAll(); syncBuiltinESMExports();
      await fs.rm(home, { recursive: true, force: true });
    }
  });
  await fs.writeFile(profile, before, { mode: 0o600 });
  for (const transport of [http, https]) context.mock.method(transport, 'request', () => {
    state.requests++; throw new Error('Unexpected network request');
  });
  context.mock.method(fs, 'open', async (path, ...args) => {
    if (path !== profile) return originalOpen(path, ...args);
    let isHeld = sample === 'open';
    if (isHeld) { state.held = true; enter(); await gate; }
    const handle = await originalOpen(path, ...args);
    handles.add(handle);
    const read = handle.readFile.bind(handle), close = handle.close.bind(handle);
    handle.close = async () => {
      try { await close(); } finally { handles.delete(handle); if (isHeld) closeHeld(); }
    };
    handle.readFile = async options => {
      state.reads++;
      const heldRead = sample === 'up-after-save' ? 3 : sample === 'up-locked' ? 2 : 1;
      if (sample !== 'open' && state.reads === heldRead) {
        isHeld = true; state.held = true; enter(); await gate;
      }
      return read(options);
    };
    return handle;
  });
  syncBuiltinESMExports();
  const service = { host: { platform: 'linux', homeDir: home, uid: 1000 }, home,
    program: [process.execPath, join(home, 'never-started.js')], environmentPath: '',
    signal: controller.signal, run: async () => { state.commands++; throw new Error('Unexpected command'); } };
  return { home, profile, before, controller, state, entered, release, heldClosed, handles, service };
}

for (const sample of ['open', 'up-initial', 'up-locked', 'up-after-save', 'client',
  'client-after-setup', 'resolve-after-setup', 'service']) {
  test(`profile cancellation during ${sample} preserves state and releases handles`,
    { timeout: 5000 }, async context => {
      const current = await fixture(context, sample);
      const { profile, controller, state, service } = current;
      const io = { stdout: assert.fail, stderr: assert.fail };
      const operation = sample.startsWith('up-')
        ? setUp('http://127.0.0.1:1', io, { configPath: profile, env: {}, service, signal: controller.signal })
        : sample.startsWith('client') ? clientDependencies(profile, io, {}, undefined,
          sample === 'client-after-setup', controller.signal)
          : sample === 'resolve-after-setup' ? resolveInstance(profile, {}, undefined, true, controller.signal)
          : sample === 'service' ? installService(service)
            : loadRunConfig(profile, {}, false, controller.signal);
      state.pending = operation.then(value => ({ value }), error => ({ error }));
      await bounded(current.entered);
      if (sample === 'up-after-save') assert.notEqual(await fs.readFile(profile, 'utf8'), current.before);
      const reason = new Error('synthetic profile cancellation');
      controller.abort(reason);
      const result = await bounded(state.pending);
      if (sample.startsWith('up-') || sample === 'resolve-after-setup') assert.equal(result.error?.code, 'aborted');
      else assert.equal(result.error, reason);
      assert.equal(await fs.readFile(profile, 'utf8'), current.before);
      current.release();
      await bounded(current.heldClosed);
      assert.equal(current.handles.size, 0);
      assert.deepEqual(await fs.readdir(current.home), ['framerize-remote.json']);
      assert.equal(state.commands, 0);
      assert.equal(state.requests, 0);
    });
}

test('responsive, missing and malformed profiles keep their existing results', async context => {
  const directory = await fs.mkdtemp(join(tmpdir(), 'frm-profile-control-'));
  context.after(() => fs.rm(directory, { recursive: true, force: true }));
  const path = join(directory, 'profile.json');
  const env = { FRM_REMOTE: 'http://127.0.0.1:1' };
  assert.equal((await loadRunConfig(path, env)).remote, env.FRM_REMOTE);
  await fs.writeFile(path, '{"remote":"http://127.0.0.1:2"}\n', { mode: 0o600 });
  assert.equal((await loadRunConfig(path, {})).remote, 'http://127.0.0.1:2');
  await fs.writeFile(path, '{');
  await assert.rejects(loadRunConfig(path, env), error => error.code === 'profile_invalid');
  const reason = new Error('synthetic pre-cancellation');
  await assert.rejects(loadRunConfig(path, env, false, AbortSignal.abort(reason)), error => error === reason);
});
