import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, readdir, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL(
  '../src/compatibility/settings.ts', import.meta.url))],
bundle: true, platform: 'node', format: 'esm', write: false });
const { writeClaudeSettings, undoClaudeSettings } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const target = { origin: 'http://127.0.0.1:18788', localToken: 'synthetic-collision-token' };
const initial = Buffer.from('{"theme":"dark"}\n');
const sentinel = Buffer.from('synthetic file owned by another writer\n');
const fixedTime = 1_800_000_000_000;
const optionalFile = async path => readFile(path).catch(error => {
  if (error.code === 'ENOENT') return null;
  throw error;
});

for (const collision of ['record', 'settings', 'none']) {
  test(`settings preserve another writer's staging file: ${collision}`, { timeout: 5000 }, async context => {
    const root = await realpath(await mkdtemp(join(tmpdir(), 'frm-settings-temp-ownership-')));
    context.after(() => rm(root, { recursive: true, force: true }));
    const state = join(root, 'state');
    const settings = join(root, 'settings.json');
    const record = join(state, 'claude-settings.json');
    await mkdir(state, { mode: 0o700 });
    await writeFile(settings, initial, { mode: 0o600 });
    const staged = collision === 'none' ? null
      : `${collision === 'record' ? record : settings}.frm-${process.pid}-${fixedTime}`;
    if (staged) await writeFile(staged, sentinel, { mode: 0o600, flag: 'wx' });
    let errorCode;
    context.mock.method(Date, 'now', () => fixedTime);
    try { await writeClaudeSettings(settings, state, target); }
    catch (error) { errorCode = error.code ?? error.name; }
    finally { context.mock.restoreAll(); }
    const stagedAfter = staged ? await optionalFile(staged) : null;
    const settingsAfter = await readFile(settings);
    const recordAfter = await optionalFile(record);
    let undoResult;
    let restoredSettings;
    if (!errorCode) {
      undoResult = await undoClaudeSettings(state);
      restoredSettings = JSON.parse(await readFile(settings, 'utf8'));
    }
    const recordRemaining = await optionalFile(record);
    const rootEntries = await readdir(root);
    const stateEntries = await readdir(state);
    await rm(root, { recursive: true, force: true });
    const removed = await readdir(root).then(() => false, error => error.code === 'ENOENT');
    context.diagnostic(JSON.stringify({ collision, errorCode: errorCode ?? null,
      stagedPreserved: staged ? stagedAfter?.equals(sentinel) ?? false : null, removed }));
    assert.equal(removed, true, 'private fixture directory was removed');
    if (errorCode) {
      assert.notEqual(collision, 'none');
      assert.equal(errorCode, 'EEXIST');
      assert.deepEqual(settingsAfter, initial);
      assert.equal(recordAfter, null, 'failed save must leave no new undo record');
    } else {
      assert.deepEqual(JSON.parse(settingsAfter), { theme: 'dark', env: {
        ANTHROPIC_BASE_URL: `${target.origin}/anthropic`, ANTHROPIC_AUTH_TOKEN: target.localToken } });
      assert.equal(JSON.parse(recordAfter).settingsPath, settings);
      assert.equal(undoResult, 'restored');
      assert.deepEqual(restoredSettings, { theme: 'dark' });
    }
    assert.equal(recordRemaining, null);
    const ownedEntries = entries => entries.filter(name => !staged || name !== basename(staged));
    assert.deepEqual(ownedEntries(rootEntries).sort(), ['settings.json', 'state']);
    assert.deepEqual(ownedEntries(stateEntries), []);
    if (staged) assert.deepEqual(stagedAfter, sentinel,
      'a failed exclusive write must not delete another writer\'s file');
  });
}
