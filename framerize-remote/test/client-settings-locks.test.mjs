import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL(
  '../src/compatibility/settings.ts', import.meta.url))],
bundle: true, platform: 'node', format: 'esm', write: false });
const bundle = compiled.outputFiles[0].contents;
const bundleSha256 = createHash('sha256').update(bundle).digest('hex');
const { writeClaudeSettings, undoClaudeSettings, SettingsError } = await import(
  `data:text/javascript;base64,${Buffer.from(bundle).toString('base64')}`);
const original = { model: 'synthetic-model', env: { KEEP: 'synthetic-value' } };
const target = { origin: 'http://127.0.0.1:18788', localToken: 'synthetic-first' };
const nextTarget = { origin: 'http://127.0.0.1:18789', localToken: 'synthetic-second' };

async function within(promise, message) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(message)), 1200);
    })]);
  } finally { clearTimeout(timer); }
}

async function fixture(context) {
  const root = await fs.realpath(await fs.mkdtemp(join(tmpdir(), 'frm-settings-locks-')));
  const operations = [];
  const releases = [];
  const restore = () => { context.mock.restoreAll(); syncBuiltinESMExports(); };
  context.after(async () => {
    for (const release of releases) release();
    try { await within(Promise.allSettled(operations), 'owned operations did not settle'); }
    finally { restore(); await fs.rm(root, { recursive: true, force: true }); }
    await assert.rejects(fs.stat(root), { code: 'ENOENT' });
    context.diagnostic(JSON.stringify({ bundleSha256, privateStateRemoved: true }));
  });
  const settings = join(root, 'settings.json');
  const state = join(root, 'state');
  await fs.writeFile(settings, JSON.stringify(original), { mode: 0o600 });
  return { root, settings, state, record: join(state, 'claude-settings.json'), releases, restore,
    track(promise) { operations.push(promise); void promise.catch(() => {}); return promise; } };
}

function holdRename(context, owned, destination) {
  const entered = Promise.withResolvers();
  const released = Promise.withResolvers();
  owned.releases.push(released.resolve);
  const rename = fs.rename;
  let held = false;
  context.mock.method(fs, 'rename', async (from, to) => {
    if (to === destination && !held) { held = true; entered.resolve(); await released.promise; }
    return rename(from, to);
  });
  syncBuiltinESMExports();
  return { entered: entered.promise, release: released.resolve };
}

async function assertNoLocks(root) {
  const entries = await fs.readdir(root, { recursive: true });
  assert.deepEqual(entries.filter(entry => entry.includes('.frm-')), []);
}

for (const shared of ['both', 'record', 'settings']) {
  test(`a concurrent writer sharing ${shared} is rejected without changing the winner`,
    { timeout: 5000 }, async context => {
      const owned = await fixture(context);
      const secondSettings = shared === 'record' ? join(owned.root, 'other.json') : owned.settings;
      const secondState = shared === 'settings' ? join(owned.root, 'other-state') : owned.state;
      if (secondSettings !== owned.settings) await fs.writeFile(secondSettings, JSON.stringify(original));
      const held = holdRename(context, owned, shared === 'record' ? owned.record : owned.settings);
      const first = owned.track(writeClaudeSettings(owned.settings, owned.state, target));
      await within(held.entered, 'first writer did not enter rename');
      await assert.rejects(within(owned.track(writeClaudeSettings(secondSettings, secondState, nextTarget)),
        'conflicting writer waited instead of rejecting'), error => error instanceof SettingsError && /locked/.test(error.message));
      assert.deepEqual(JSON.parse(await fs.readFile(secondSettings, 'utf8')), original);
      if (secondState !== owned.state) await assert.rejects(fs.stat(join(secondState, 'claude-settings.json')), { code: 'ENOENT' });
      held.release();
      await within(first, 'first writer did not finish');
      owned.restore();
      await assertNoLocks(owned.root);
      assert.equal(await undoClaudeSettings(owned.state), 'restored');
      assert.deepEqual(JSON.parse(await fs.readFile(owned.settings, 'utf8')), original);
      await assertNoLocks(owned.root);
    });
}

for (const active of ['save', 'undo']) {
  test(`an active ${active} cannot overlap its reverse operation`, { timeout: 5000 }, async context => {
    const owned = await fixture(context);
    await writeClaudeSettings(owned.settings, owned.state, target);
    const held = holdRename(context, owned, owned.settings);
    const first = owned.track(active === 'save'
      ? writeClaudeSettings(owned.settings, owned.state, nextTarget) : undoClaudeSettings(owned.state));
    await within(held.entered, 'active operation did not enter rename');
    const before = await fs.readFile(owned.settings);
    const recordBefore = await fs.readFile(owned.record);
    const second = owned.track(active === 'save'
      ? undoClaudeSettings(owned.state) : writeClaudeSettings(owned.settings, owned.state, nextTarget));
    await assert.rejects(within(second, 'reverse operation did not reject'),
      error => error instanceof SettingsError && /locked/.test(error.message));
    assert.deepEqual(await fs.readFile(owned.settings), before);
    assert.deepEqual(await fs.readFile(owned.record), recordBefore);
    held.release();
    await within(first, 'active operation did not finish');
    owned.restore();
    await assertNoLocks(owned.root);
    if (active === 'undo') await writeClaudeSettings(owned.settings, owned.state, nextTarget);
    assert.equal(await undoClaudeSettings(owned.state), 'restored');
    assert.deepEqual(JSON.parse(await fs.readFile(owned.settings, 'utf8')), original);
    await assertNoLocks(owned.root);
  });
}

test('a failed write restores its record and releases both locks for retry', { timeout: 5000 }, async context => {
  const owned = await fixture(context);
  await writeClaudeSettings(owned.settings, owned.state, target);
  const before = await fs.readFile(owned.settings);
  const recordBefore = await fs.readFile(owned.record);
  const failure = Object.assign(new Error('Synthetic rename failure'), { code: 'EACCES' });
  const rename = fs.rename;
  context.mock.method(fs, 'rename', (from, to) => to === owned.settings
    ? Promise.reject(failure) : rename(from, to));
  syncBuiltinESMExports();
  await assert.rejects(writeClaudeSettings(owned.settings, owned.state, nextTarget), error => error === failure);
  owned.restore();
  assert.deepEqual(await fs.readFile(owned.settings), before);
  assert.deepEqual(await fs.readFile(owned.record), recordBefore);
  await assertNoLocks(owned.root);
  await writeClaudeSettings(owned.settings, owned.state, nextTarget);
  assert.equal(await undoClaudeSettings(owned.state), 'restored');
  assert.deepEqual(JSON.parse(await fs.readFile(owned.settings, 'utf8')), original);
  await assertNoLocks(owned.root);
});

for (const alias of ['state', 'settings']) {
  test(`canonical ${alias} paths share the same lock`,
    { timeout: 5000, skip: process.platform === 'win32' }, async context => {
      const owned = await fixture(context);
      await fs.mkdir(owned.state);
      const link = join(owned.root, 'alias');
      await fs.symlink(alias === 'state' ? owned.state : owned.settings, link);
      const secondSettings = alias === 'settings' ? link : join(owned.root, 'other.json');
      const secondState = alias === 'state' ? link : join(owned.root, 'other-state');
      const held = holdRename(context, owned, alias === 'state' ? owned.record : owned.settings);
      const first = owned.track(writeClaudeSettings(owned.settings, owned.state, target));
      await within(held.entered, 'first writer did not reach rename');
      await assert.rejects(within(owned.track(writeClaudeSettings(secondSettings, secondState, nextTarget)),
        'alias writer did not reject'), error => error instanceof SettingsError && /locked/.test(error.message));
      held.release();
      await within(first, 'first writer did not finish');
      owned.restore();
      assert.equal((await fs.lstat(link)).isSymbolicLink(), true);
      assert.equal(await undoClaudeSettings(owned.state), 'restored');
      assert.deepEqual(JSON.parse(await fs.readFile(owned.settings, 'utf8')), original);
      await assertNoLocks(owned.root);
    });
}

for (const locked of ['record', 'settings']) {
  test(`an existing ${locked} lock is not removed or overwritten`, { timeout: 5000 }, async context => {
    const owned = await fixture(context);
    await fs.mkdir(owned.state);
    const lock = `${owned[locked]}.frm-lock`;
    await fs.mkdir(lock);
    const sentinel = join(lock, 'other-writer');
    await fs.writeFile(sentinel, 'synthetic owner');
    await assert.rejects(writeClaudeSettings(owned.settings, owned.state, target),
      error => error instanceof SettingsError && error.message.includes(lock));
    assert.equal(await fs.readFile(sentinel, 'utf8'), 'synthetic owner');
    assert.deepEqual(JSON.parse(await fs.readFile(owned.settings, 'utf8')), original);
    await assert.rejects(fs.stat(owned.record), { code: 'ENOENT' });
    await fs.rm(lock, { recursive: true });
    await assertNoLocks(owned.root);
    await writeClaudeSettings(owned.settings, owned.state, target);
    assert.equal(await undoClaudeSettings(owned.state), 'restored');
    await assertNoLocks(owned.root);
  });
}

test('undo without a record does not create state or locks', { timeout: 5000 }, async context => {
  const owned = await fixture(context);
  assert.equal(await undoClaudeSettings(owned.state), 'not_configured');
  await assert.rejects(fs.stat(owned.state), { code: 'ENOENT' });
  assert.deepEqual(JSON.parse(await fs.readFile(owned.settings, 'utf8')), original);
  await assertNoLocks(owned.root);
});

test('failed undo releases locks and keeps its record for retry', { timeout: 5000 }, async context => {
  const owned = await fixture(context);
  await writeClaudeSettings(owned.settings, owned.state, target);
  const before = await fs.readFile(owned.settings);
  const recordBefore = await fs.readFile(owned.record);
  const failure = new Error('Synthetic undo failure');
  const rename = fs.rename;
  context.mock.method(fs, 'rename', (from, to) => to === owned.settings
    ? Promise.reject(failure) : rename(from, to));
  syncBuiltinESMExports();
  await assert.rejects(undoClaudeSettings(owned.state), error => error === failure);
  owned.restore();
  assert.deepEqual(await fs.readFile(owned.settings), before);
  assert.deepEqual(await fs.readFile(owned.record), recordBefore);
  await assertNoLocks(owned.root);
  assert.equal(await undoClaudeSettings(owned.state), 'restored');
  assert.deepEqual(JSON.parse(await fs.readFile(owned.settings, 'utf8')), original);
  await assertNoLocks(owned.root);
});
