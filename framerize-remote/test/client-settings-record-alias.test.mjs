import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, readlink, rm, symlink, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { writeClaudeSettings, undoClaudeSettings, SettingsError } from './settings.ts';",
  resolveDir: fileURLToPath(new URL('../src/compatibility/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false });
const bytes = compiled.outputFiles[0].contents;
const bundleSha256 = createHash('sha256').update(bytes).digest('hex');
const { writeClaudeSettings, undoClaudeSettings, SettingsError } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
const target = { origin: 'http://127.0.0.1:8788', localToken: 'synthetic-record-alias-token' };
const original = '{ "theme": "synthetic", "env": { "UNRELATED": "keep" } }\n';

for (const variant of ['direct', 'symlink', 'separate']) {
  test(`settings persistence protects the undo record destination: ${variant}`,
    { timeout: 3000, skip: variant === 'symlink' && process.platform === 'win32' }, async context => {
      const directory = await mkdtemp(join(tmpdir(), 'frm-record-alias-'));
      const cleanup = () => rm(directory, { recursive: true, force: true });
      context.after(cleanup);
      const stateDir = join(directory, 'state'), clientDir = join(directory, 'client');
      await mkdir(stateDir, { mode: 0o700 });
      await mkdir(clientDir, { mode: 0o700 });
      const recordPath = join(stateDir, 'claude-settings.json');
      const settingsPath = variant === 'direct' ? recordPath : join(clientDir, 'settings.json');
      await writeFile(variant === 'symlink' ? recordPath : settingsPath, original, { mode: 0o600 });
      if (variant === 'symlink') await symlink(recordPath, settingsPath);
      let rejection;
      try { await writeClaudeSettings(settingsPath, stateDir, target); }
      catch (error) { rejection = error; }
      const afterWrite = await readFile(settingsPath, 'utf8');
      const parsed = JSON.parse(afterWrite);
      const undo = rejection ? undefined : await undoClaudeSettings(stateDir);
      const afterUndo = await readFile(settingsPath, 'utf8');
      const linkUnchanged = variant !== 'symlink' || await readlink(settingsPath) === recordPath;
      const observations = { variant, bundleSha256, rejected: rejection !== undefined,
        managedValuesPresent: parsed.env?.ANTHROPIC_AUTH_TOKEN === target.localToken,
        hasUndoMetadata: typeof parsed.settingsPath === 'string' && typeof parsed.written === 'object',
        originalUnchanged: afterWrite === original, undo, linkUnchanged };
      await cleanup();
      await assert.rejects(readFile(settingsPath), { code: 'ENOENT' });
      context.diagnostic(JSON.stringify({ ...observations, privateStateRemoved: true }));
      if (variant === 'separate') {
        assert.equal(rejection, undefined);
        assert.equal(undo, 'restored');
        assert.equal(observations.managedValuesPresent, true);
        assert.deepEqual(JSON.parse(afterUndo), JSON.parse(original));
      } else {
        assert.ok(rejection instanceof SettingsError, 'refuse a settings destination that is also its undo record');
        assert.equal(afterWrite, original);
        assert.equal(afterUndo, original);
        assert.equal(linkUnchanged, true);
      }
    });
}
