import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, readdir, readlink, realpath, rm, symlink } from 'node:fs/promises';
import fs from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join, parse } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/settings.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { writeClaudeSettings, undoClaudeSettings, SettingsError } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const target = { origin: 'http://127.0.0.1:8788', localToken: 'synthetic-parent-alias-token' };

test('a missing filesystem root is refused without repeating parent resolution', async context => {
  const root = parse(process.cwd()).root;
  const missing = Object.assign(new Error('Synthetic missing root'), { code: 'ENOENT' });
  let queries = 0;
  context.mock.method(fs, 'realpath', async path => {
    assert.equal(path, root);
    if (++queries > 2) throw new Error('Parent resolution repeated at the root');
    throw missing;
  });
  context.mock.method(fs, 'lstat', async path => { assert.equal(path, root); throw missing; });
  syncBuiltinESMExports();
  try {
    await assert.rejects(writeClaudeSettings(root, join(root, 'synthetic-state'), target), SettingsError);
  } finally {
    context.mock.restoreAll();
    syncBuiltinESMExports();
  }
  assert.equal(queries, 1);
});

for (const separate of [false, true]) {
  test(`missing settings below a parent alias: ${separate ? 'distinct destinations' : 'undo record collision'}`,
    { timeout: 5000 }, async context => {
      const root = await realpath(await mkdtemp(join(tmpdir(), 'frm-settings-parent-alias-')));
      context.after(() => rm(root, { recursive: true, force: true }));
      const actual = join(root, 'actual'), alias = join(root, 'alias');
      await mkdir(actual);
      await symlink(actual, alias, process.platform === 'win32' ? 'junction' : 'dir');
      const originalLink = await readlink(alias);
      const state = join(alias, 'new', 'state');
      const settings = separate ? join(actual, 'new', 'settings.json')
        : join(actual, 'new', 'state', 'claude-settings.json');
      let rejection, record, updated, undo, restored;
      try { await writeClaudeSettings(settings, state, target); }
      catch (error) { rejection = error; }
      if (separate && !rejection) {
        await writeClaudeSettings(join(alias, 'new', 'settings.json'), state,
          { ...target, origin: 'http://127.0.0.1:8789' });
        record = JSON.parse(await readFile(join(state, 'claude-settings.json'), 'utf8'));
        updated = JSON.parse(await readFile(settings, 'utf8'));
        undo = await undoClaudeSettings(join(actual, 'new', 'state'));
        restored = JSON.parse(await readFile(settings, 'utf8'));
      }
      const remaining = await readdir(actual);
      const unchangedLink = await readlink(alias) === originalLink;
      await rm(root, { recursive: true, force: true });
      await assert.rejects(readdir(root), { code: 'ENOENT' });
      assert.equal(unchangedLink, true);
      if (separate) {
        assert.equal(rejection, undefined);
        assert.equal(record.settingsPath, settings);
        assert.equal(updated.env.ANTHROPIC_BASE_URL, 'http://127.0.0.1:8789/anthropic');
        assert.equal(undo, 'restored');
        assert.deepEqual(restored, {});
      } else {
        assert.ok(rejection instanceof SettingsError);
        assert.deepEqual(remaining, [], 'refusal must not create either destination or its parents');
      }
    });
}
