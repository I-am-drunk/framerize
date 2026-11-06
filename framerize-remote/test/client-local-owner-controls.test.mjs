import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { getEventListeners } from 'node:events';
import { setImmediate as nextTurn } from 'node:timers/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/localSettings.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { claudeLocalSettingsPath } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const options = { timeout: 6000, skip: process.platform === 'win32' };
const uid = process.getuid?.();

function deferred() {
  let resolve, reject;
  const promise = new Promise((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}

async function bounded(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolveUnused, reject) => {
      timer = setTimeout(() => reject(new Error('Fixture did not settle')), 1500);
    })]);
  } finally { clearTimeout(timer); }
}

async function fixture(context) {
  const root = await fs.mkdtemp(join(tmpdir(), 'local-owner-controls-'));
  context.after(async () => {
    await fs.rm(root, { recursive: true, force: true });
    await assert.rejects(fs.stat(root), { code: 'ENOENT' });
  });
  const checkout = join(root, 'main');
  const cwd = join(root, 'linked');
  const controller = new AbortController();
  const captures = [];
  const deps = { platform: process.platform, home: join(root, 'home'),
    signal: controller.signal, realPath: path => path,
    runner: { async capture(command) {
      captures.push(command);
      assert.equal(command.command, 'git');
      return `worktree ${checkout}\0HEAD synthetic\0\0`;
    } }, fileOwner: async () => uid };
  return { checkout, cwd, controller, captures, deps };
}

for (const suffix of ['', '.git', '.claude']) {
  test(`cancels held ${suffix || 'root'} ownership and consumes its late failure`, options, async context => {
    const state = await fixture(context);
    const held = deferred(), entered = deferred();
    context.after(() => held.resolve(uid));
    state.deps.fileOwner = async path => {
      if (path === join(state.checkout, suffix)) { entered.resolve(); return held.promise; }
      return uid;
    };
    const operation = claudeLocalSettingsPath(state.deps, state.cwd)
      .then(value => ({ value }), error => ({ error }));
    await bounded(entered.promise);
    const reason = new Error('synthetic cancellation reason');
    state.controller.abort(reason);
    const result = await bounded(operation);
    assert.equal(result.error, reason);
    assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
    held.reject(new Error('synthetic late ownership failure'));
    await nextTurn();
    assert.equal(state.captures.length, 1);
  });
}

test('ownership errors preserve their identity and remove the abort listener', options, async context => {
  const state = await fixture(context);
  const failure = new Error('synthetic ownership failure');
  state.deps.fileOwner = async () => { throw failure; };
  await assert.rejects(claudeLocalSettingsPath(state.deps, state.cwd), error => error === failure);
  assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
});

test('ownership fallback works without a signal', options, async context => {
  const state = await fixture(context);
  delete state.deps.signal;
  state.deps.fileOwner = async path => path === join(state.checkout, '.git') ? null : uid;
  assert.equal(await claudeLocalSettingsPath(state.deps, state.cwd),
    join(state.cwd, '.claude', 'settings.local.json'));
});

test('pre-aborted lookup starts no discovery or ownership work', options, async context => {
  const state = await fixture(context);
  const reason = new Error('synthetic pre-abort');
  let owners = 0;
  state.deps.fileOwner = async () => { owners++; return uid; };
  state.controller.abort(reason);
  await assert.rejects(claudeLocalSettingsPath(state.deps, state.cwd), error => error === reason);
  assert.equal(owners, 0);
  assert.equal(state.captures.length, 0);
  assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
});
