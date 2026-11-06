import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { isAbsolute, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/settings.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { claudeSettingsPath, writeClaudeSettings, undoClaudeSettings } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const target = { origin: 'http://127.0.0.1:8788', localToken: 'synthetic-relative-settings' };

for (const existed of [false, true]) {
  test(`undo ${existed ? 'existing' : 'new'} relative settings from another directory`,
    { timeout: 5000, concurrency: false }, async context => {
      const previousDirectory = process.cwd();
      const root = await realpath(await mkdtemp(join(tmpdir(), 'frm-relative-settings-')));
      context.after(async () => {
        process.chdir(previousDirectory);
        await rm(root, { recursive: true, force: true });
        await assert.rejects(readFile(join(root, 'state', 'claude-settings.json')), { code: 'ENOENT' });
        context.diagnostic('Working directory restored; private state removed.');
      });
      const first = join(root, 'first');
      const second = join(root, 'second');
      const state = join(root, 'state');
      await mkdir(join(first, 'config'), { recursive: true });
      await mkdir(second);
      const file = join(first, 'config', 'settings.json');
      const original = existed ? { model: 'synthetic-model' } : {};
      if (existed) await writeFile(file, JSON.stringify(original));
      process.chdir(first);
      const path = claudeSettingsPath({ CLAUDE_CONFIG_DIR: 'config' }, first);
      await writeClaudeSettings(path, state, target);
      const record = JSON.parse(await readFile(join(state, 'claude-settings.json'), 'utf8'));
      process.chdir(second);
      const outcome = await undoClaudeSettings(state);
      const after = JSON.parse(await readFile(file, 'utf8'));
      context.diagnostic(JSON.stringify({ existed, absoluteRecord: isAbsolute(record.settingsPath),
        outcome, stillManaged: after.env?.ANTHROPIC_BASE_URL !== undefined }));
      await assert.rejects(readFile(join(second, 'config', 'settings.json')), { code: 'ENOENT' });
      assert.equal(outcome, 'restored');
      assert.equal(record.settingsPath, file);
      assert.deepEqual(after, original);
      await assert.rejects(readFile(join(state, 'claude-settings.json')), { code: 'ENOENT' });
    });
}
