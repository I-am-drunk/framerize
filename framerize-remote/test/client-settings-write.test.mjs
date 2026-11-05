import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/compatibility/settings.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
const { writeClaudeSettings, undoClaudeSettings, SettingsError } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const target = { origin: 'http://127.0.0.1:8788', localToken: 'synthetic-original' };

async function fixture(t) {
  const root = await fs.mkdtemp(join(tmpdir(), 'frm-settings-write-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  return { root, file: join(root, 'settings.json'), state: join(root, 'state'),
    record: join(root, 'state', 'claude-settings.json') };
}

function rejectRename(t, destination) {
  const rename = fs.rename;
  const failure = Object.assign(new Error('Synthetic settings write failure'), { code: 'EACCES' });
  t.mock.method(fs, 'rename', async (from, to) => {
    if (to === destination) throw failure;
    return rename(from, to);
  });
  const restore = () => { t.mock.restoreAll(); syncBuiltinESMExports(); };
  t.after(restore);
  syncBuiltinESMExports();
  return { failure, restore };
}

test('a failed settings update preserves the previous undo record', async t => {
  const { root, file, state, record } = await fixture(t);
  const original = { model: 'synthetic-model', env: { KEEP: 'yes' } };
  await fs.writeFile(file, JSON.stringify(original));
  await writeClaudeSettings(file, state, target);
  const previousSettings = await fs.readFile(file, 'utf8');
  const previousRecord = await fs.readFile(record, 'utf8');
  const fault = rejectRename(t, file);
  await assert.rejects(writeClaudeSettings(file, state, {
    origin: 'http://127.0.0.1:8789', localToken: 'synthetic-next',
  }), error => error === fault.failure);
  fault.restore();
  assert.equal(await fs.readFile(file, 'utf8'), previousSettings);
  assert.equal(await fs.readFile(record, 'utf8'), previousRecord);
  assert.deepEqual((await fs.readdir(root)).sort(), ['settings.json', 'state']);
  assert.deepEqual(await fs.readdir(state), ['claude-settings.json']);
  assert.equal(await undoClaudeSettings(state), 'restored');
  assert.deepEqual(JSON.parse(await fs.readFile(file, 'utf8')), original);
  await assert.rejects(fs.stat(record), { code: 'ENOENT' });
});

test('a failed first settings write leaves no undo record or temporary file', async t => {
  const { root, file, state, record } = await fixture(t);
  const fault = rejectRename(t, file);
  await assert.rejects(writeClaudeSettings(file, state, target), error => error === fault.failure);
  fault.restore();
  await assert.rejects(fs.stat(file), { code: 'ENOENT' });
  await assert.rejects(fs.stat(record), { code: 'ENOENT' });
  assert.deepEqual(await fs.readdir(root), ['state']);
  assert.deepEqual(await fs.readdir(state), []);
  assert.equal(await undoClaudeSettings(state), 'not_configured');
});

test('an unavailable undo record destination leaves existing settings untouched', async t => {
  const { file, state, record } = await fixture(t);
  const original = '{"model":"synthetic-model"}\n';
  await fs.writeFile(file, original);
  const fault = rejectRename(t, record);
  await assert.rejects(writeClaudeSettings(file, state, target), error => error === fault.failure);
  fault.restore();
  assert.equal(await fs.readFile(file, 'utf8'), original);
  await assert.rejects(fs.stat(record), { code: 'ENOENT' });
  assert.deepEqual(await fs.readdir(state), []);
});

test('a failed undo-record restoration reports both write failures', async t => {
  const { root, file, state, record } = await fixture(t);
  await writeClaudeSettings(file, state, target);
  const previousSettings = await fs.readFile(file, 'utf8');
  const writeError = new Error('Synthetic settings failure');
  const restoreError = new Error('Synthetic restoration failure');
  const rename = fs.rename;
  let settingsFailed = false;
  t.mock.method(fs, 'rename', async (from, to) => {
    if (to === file) { settingsFailed = true; throw writeError; }
    if (settingsFailed && to === record) throw restoreError;
    return rename(from, to);
  });
  t.after(() => { t.mock.restoreAll(); syncBuiltinESMExports(); });
  syncBuiltinESMExports();
  await assert.rejects(writeClaudeSettings(file, state, { ...target, localToken: 'synthetic-next' }), error => {
    assert.ok(error instanceof SettingsError);
    assert.match(error.message, /undo record could not be restored/);
    assert.ok(error.cause instanceof AggregateError);
    assert.deepEqual(error.cause.errors, [writeError, restoreError]);
    return true;
  });
  assert.equal(await fs.readFile(file, 'utf8'), previousSettings);
  assert.deepEqual((await fs.readdir(root)).sort(), ['settings.json', 'state']);
  assert.deepEqual(await fs.readdir(state), ['claude-settings.json']);
});
