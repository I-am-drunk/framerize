import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/settings.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { writeClaudeSettings, undoClaudeSettings, SettingsError } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const target = { origin: 'http://127.0.0.1:8788', localToken: 'synthetic-settings-target-token' };

async function fixture(context) {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'frm-settings-target-')));
  context.after(async () => {
    await rm(root, { recursive: true, force: true });
    await assert.rejects(readFile(join(root, 'first.json')), { code: 'ENOENT' });
    context.diagnostic('Private fixture removed.');
  });
  return { first: join(root, 'first.json'), second: join(root, 'second.json'),
    state: join(root, 'state'), record: join(root, 'state', 'claude-settings.json') };
}

test('changing settings target cannot orphan the existing undo record', { timeout: 5000 }, async context => {
  const files = await fixture(context);
  const originalFirst = '{"model":"synthetic-first"}\n';
  const originalSecond = '{"model":"synthetic-second"}\n';
  await writeFile(files.first, originalFirst);
  await writeFile(files.second, originalSecond);
  await writeClaudeSettings(files.first, files.state, target);
  const savedFirst = await readFile(files.first, 'utf8');
  const savedRecord = await readFile(files.record, 'utf8');
  let rejection;
  try { await writeClaudeSettings(files.second, files.state, { ...target, origin: 'http://127.0.0.1:8789' }); }
  catch (error) { rejection = error; }
  const recordPreserved = await readFile(files.record, 'utf8') === savedRecord;
  const firstPreserved = await readFile(files.first, 'utf8') === savedFirst;
  const secondPreserved = await readFile(files.second, 'utf8') === originalSecond;
  const undo = await undoClaudeSettings(files.state);
  const firstAfterUndo = JSON.parse(await readFile(files.first, 'utf8'));
  const firstStillManaged = firstAfterUndo.env?.ANTHROPIC_BASE_URL !== undefined;
  context.diagnostic(JSON.stringify({ rejected: !!rejection, recordPreserved, firstPreserved,
    secondPreserved, undo, firstStillManaged }));
  assert.ok(rejection instanceof SettingsError, 'Refuse a different settings target until the prior write is undone.');
  assert.match(rejection.message, /--undo/);
  assert.equal(recordPreserved, true);
  assert.equal(firstPreserved, true);
  assert.equal(secondPreserved, true);
  assert.equal(undo, 'restored');
  assert.deepEqual(firstAfterUndo, JSON.parse(originalFirst));
  await assert.rejects(readFile(files.record), { code: 'ENOENT' });
});

test('relative and absolute names for the same settings file remain updateable', { timeout: 5000 }, async context => {
  const files = await fixture(context);
  const localPath = relative(process.cwd(), files.first);
  await writeClaudeSettings(localPath, files.state, target);
  await writeClaudeSettings(files.first, files.state, { ...target, origin: 'http://127.0.0.1:8789' });
  const updated = JSON.parse(await readFile(files.first, 'utf8'));
  assert.equal(updated.env.ANTHROPIC_BASE_URL, 'http://127.0.0.1:8789/anthropic');
  assert.equal(await undoClaudeSettings(files.state), 'restored');
  assert.deepEqual(JSON.parse(await readFile(files.first, 'utf8')), {});
  await assert.rejects(readFile(files.record), { code: 'ENOENT' });
});
