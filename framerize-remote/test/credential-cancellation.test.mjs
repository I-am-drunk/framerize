import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
const root = fileURLToPath(new URL('../../', import.meta.url));
async function load(name) {
  const output = await build({ entryPoints: [join(root, 'framerize-remote/src', name)],
    bundle: true, write: false, platform: 'node', format: 'esm',
    alias: { '@framerize/core': join(root, 'framerize-core/src/index.ts') } });
  return import(`data:text/javascript;base64,${Buffer.from(output.outputFiles[0].contents).toString('base64')}`);
}
const { loadCredential } = await load('credentials.ts');
const { clientDependencies } = await load('clientCli.ts');

async function bounded(pending) {
  let timer;
  try { return await Promise.race([pending, new Promise((unused, reject) => {
    timer = setTimeout(() => reject(new Error('Owned operation exceeded one second')), 1000);
  })]); } finally { clearTimeout(timer); }
}

async function controlledFile(context, phase) {
  const directory = await fs.mkdtemp(join(tmpdir(), 'frm-credential-signal-'));
  const token = join(directory, 'token'), profile = join(directory, 'profile.json');
  const controller = new AbortController();
  const originalOpen = fs.open;
  let enter, release, finish;
  const entered = new Promise(resolve => { enter = resolve; });
  const gate = new Promise(resolve => { release = resolve; });
  const closed = new Promise(resolve => { finish = resolve; });
  const handles = new Set();
  const state = { started: false, pending: undefined };
  context.after(async () => {
    release();
    controller.abort();
    try {
      if (state.pending) await bounded(state.pending);
      if (state.started) await bounded(closed);
    } finally {
      fs.open = originalOpen;
      syncBuiltinESMExports();
      await Promise.all([...handles].map(handle => handle.close()));
      await fs.rm(directory, { recursive: true, force: true });
    }
  });
  await fs.writeFile(token, 'synthetic-credential\n', { mode: 0o600 });
  const before = JSON.stringify({ remote: 'http://127.0.0.1:1',
    localToken: { kind: 'file', path: token } });
  await fs.writeFile(profile, before, { mode: 0o600 });
  fs.open = async (path, ...args) => {
    if (path !== token) return originalOpen(path, ...args);
    state.started = true;
    if (phase === 'open') { enter(); await gate; }
    const handle = await originalOpen(path, ...args);
    handles.add(handle);
    const close = handle.close.bind(handle), read = handle.read.bind(handle);
    handle.close = async () => {
      try { await close(); } finally { handles.delete(handle); finish(); }
    };
    if (phase !== 'open') handle.read = async (...parameters) => {
      enter(); await gate; return read(...parameters);
    };
    return handle;
  };
  syncBuiltinESMExports();
  return { token, profile, before, controller, entered, release, closed, handles, state };
}

for (const phase of ['open', 'read', 'client']) {
  test(`credential cancellation interrupts ${phase} and closes acquired handles`,
    { timeout: 5000 }, async context => {
      const fixture = await controlledFile(context, phase);
      const { controller, state, token, profile } = fixture;
      const operation = phase === 'client'
        ? clientDependencies(profile, { stdout: assert.fail, stderr: assert.fail }, {}, undefined, controller.signal)
        : loadCredential({ kind: 'file', path: token }, {}, controller.signal);
      state.pending = operation.then(value => ({ value }), error => ({ error }));
      await bounded(fixture.entered);
      const reason = new Error('synthetic cancellation');
      controller.abort(reason);
      assert.equal((await bounded(state.pending)).error, reason);
      fixture.release();
      await bounded(fixture.closed);
      assert.equal(fixture.handles.size, 0);
      assert.equal(await fs.readFile(profile, 'utf8'), fixture.before);
      assert.equal(await fs.readFile(token, 'utf8'), 'synthetic-credential\n');
    });
}

test('already cancelled reads retain the reason for every credential kind', async () => {
  const reason = new Error('synthetic pre-cancellation');
  for (const reference of [{ kind: 'value', value: 'synthetic' },
    { kind: 'env', name: 'SYNTHETIC' }, { kind: 'file', path: '/never-opened' }]) {
    await assert.rejects(loadCredential(reference, { SYNTHETIC: 'synthetic' },
      AbortSignal.abort(reason)), error => error === reason);
  }
});
