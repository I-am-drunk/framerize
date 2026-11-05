import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, rename, rm, symlink, lstat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/compatibility/settings.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
const { writeClaudeSettings, undoClaudeSettings, SettingsError } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const target = { origin: 'http://127.0.0.1:8788', localToken: 'synthetic-local-token' };

async function fixture(t) {
  const root = await mkdtemp(join(tmpdir(), 'frm-settings-files-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  return { file: join(root, 'settings.json'), actual: join(root, 'dotfiles.json'), state: join(root, 'state') };
}

test('persist and undo preserve a settings symlink and edit its target', async t => {
  const { file, actual, state } = await fixture(t);
  const original = { model: 'opus', env: { KEEP_ME: 'yes' } };
  await writeFile(actual, JSON.stringify(original));
  await symlink(actual, file, 'file');
  await writeClaudeSettings(file, state, target);
  assert.equal((await lstat(file)).isSymbolicLink(), true);
  assert.equal(JSON.parse(await readFile(actual, 'utf8')).env.ANTHROPIC_AUTH_TOKEN, target.localToken);
  assert.equal(await undoClaudeSettings(state), 'restored');
  assert.equal((await lstat(file)).isSymbolicLink(), true);
  assert.deepEqual(JSON.parse(await readFile(actual, 'utf8')), original);
});

test('undo follows a settings symlink created after persist', async t => {
  const { file, actual, state } = await fixture(t);
  await writeClaudeSettings(file, state, target);
  await rename(file, actual);
  await symlink(actual, file, 'file');
  assert.equal(await undoClaudeSettings(state), 'restored');
  assert.equal((await lstat(file)).isSymbolicLink(), true);
  assert.deepEqual(JSON.parse(await readFile(actual, 'utf8')), {});
});

test('undo of a missing settings file reports not configured and forgets the record', async t => {
  const { file, state } = await fixture(t);
  await writeClaudeSettings(file, state, target);
  await rm(file);
  assert.equal(await undoClaudeSettings(state), 'not_configured');
  await assert.rejects(lstat(file), { code: 'ENOENT' });
  await assert.rejects(lstat(join(state, 'claude-settings.json')), { code: 'ENOENT' });
});

test('persist leaves a dangling settings symlink and its missing target untouched', async t => {
  const { file, actual, state } = await fixture(t);
  await symlink(actual, file, 'file');
  await assert.rejects(writeClaudeSettings(file, state, target), SettingsError);
  assert.equal((await lstat(file)).isSymbolicLink(), true);
  await assert.rejects(lstat(actual), { code: 'ENOENT' });
  await assert.rejects(lstat(join(state, 'claude-settings.json')), { code: 'ENOENT' });
});
