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
const hash = value => createHash('sha256').update(value).digest('hex');
const bundle = compiled.outputFiles[0].contents;
const { writeClaudeSettings, undoClaudeSettings, MANAGED_KEYS } = await import(
  `data:text/javascript;base64,${Buffer.from(bundle).toString('base64')}`);

async function within(promise, milliseconds, message) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(message)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

test('overlapping settings saves keep the final values undoable', { timeout: 5000 }, async context => {
  const root = await fs.realpath(await fs.mkdtemp(join(tmpdir(), 'frm-settings-transaction-')));
  const entered = Promise.withResolvers();
  const release = Promise.withResolvers();
  const operations = [];
  let releaseTimer;
  const restore = () => { context.mock.restoreAll(); syncBuiltinESMExports(); };
  const cleanup = async () => {
    clearTimeout(releaseTimer);
    release.resolve();
    try { await within(Promise.allSettled(operations), 1000, 'owned saves did not finish'); }
    finally { restore(); await fs.rm(root, { recursive: true, force: true }); }
  };
  context.after(cleanup);
  const settings = join(root, 'settings.json');
  const state = join(root, 'state');
  const record = join(state, 'claude-settings.json');
  const original = { model: 'synthetic-model', env: { KEEP: 'synthetic-value' } };
  await fs.writeFile(settings, `${JSON.stringify(original)}\n`, { mode: 0o600 });
  const rename = fs.rename;
  let settingsWrites = 0;
  context.mock.method(fs, 'rename', async (from, to) => {
    if (to === settings && ++settingsWrites === 1) {
      entered.resolve();
      await release.promise;
    }
    return rename(from, to);
  });
  syncBuiltinESMExports();
  const first = writeClaudeSettings(settings, state, {
    origin: 'http://127.0.0.1:18788', localToken: 'synthetic-first' });
  operations.push(first);
  void first.catch(() => {});
  await within(Promise.race([entered.promise, first.then(() => {
    throw new Error('first save completed without entering the settings rename');
  })]), 1000, 'first save did not reach the settings rename');
  const second = writeClaudeSettings(settings, state, {
    origin: 'http://127.0.0.1:18789', localToken: 'synthetic-second' });
  operations.push(second);
  void second.then(release.resolve, release.resolve);
  releaseTimer = setTimeout(release.resolve, 200);
  const outcomes = await within(Promise.allSettled(operations), 2500, 'overlapping saves did not settle');
  clearTimeout(releaseTimer);
  restore();
  const saved = JSON.parse(await fs.readFile(settings, 'utf8'));
  const undoRecord = JSON.parse(await fs.readFile(record, 'utf8'));
  const recordMatches = MANAGED_KEYS.every(key => undoRecord.written[key] === hash(saved.env[key]));
  const undoResult = await undoClaudeSettings(state);
  const restored = JSON.parse(await fs.readFile(settings, 'utf8'));
  const remaining = await fs.readdir(state);
  const rootEntries = await fs.readdir(root);
  await cleanup();
  const removed = await fs.stat(root).then(() => false, error => error.code === 'ENOENT');
  context.diagnostic(JSON.stringify({ node: process.version, bundleSha256: hash(bundle),
    outcomes: outcomes.map(outcome => outcome.status), settingsWrites, recordMatches,
    undoResult, managedValuesRemain: MANAGED_KEYS.some(key => restored.env?.[key] !== undefined),
    privateStateRemoved: removed }));
  assert.equal(removed, true);
  assert.ok(outcomes.some(outcome => outcome.status === 'fulfilled'));
  assert.equal(saved.model, original.model);
  assert.equal(saved.env.KEEP, original.env.KEEP);
  assert.equal(undoRecord.settingsPath, settings);
  assert.deepEqual(rootEntries.sort(), ['settings.json', 'state']);
  assert.deepEqual(remaining, []);
  assert.equal(recordMatches, true, 'the final successful save must match its undo record');
  assert.equal(undoResult, 'restored');
  assert.deepEqual(restored, original);
});
