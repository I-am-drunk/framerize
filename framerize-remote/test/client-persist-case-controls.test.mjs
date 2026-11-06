import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { access, mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { build, stop } from 'esbuild';

const compiled = await build({
  stdin: { contents: "export * from './settings.ts'; export * from './conflicts.ts';",
    resolveDir: fileURLToPath(new URL('../src/compatibility/', import.meta.url)), loader: 'ts' },
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
});
stop();
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const sources = Object.fromEntries(await Promise.all(Object.keys(compiled.metafile.inputs)
  .filter(path => path !== '<stdin>').sort().map(async path => [path, hash(await readFile(path))])));
const bytes = compiled.outputFiles[0].contents;
console.log(JSON.stringify({ runtime: process.version, sources, bundle: hash(bytes) }));
const { writeClaudeSettings, undoClaudeSettings, inspectClaudeSettings, SettingsError } =
  await import('data:text/javascript;base64,' + Buffer.from(bytes).toString('base64'));
const target = { origin: 'http://127.0.0.1:8788', localToken: 'synthetic-original-token' };
const next = { origin: 'http://127.0.0.1:8789', localToken: 'synthetic-next-token' };

async function fixture(context) {
  const root = await mkdtemp(join(tmpdir(), 'frm-persist-controls-'));
  context.after(async () => {
    await rm(root, { recursive: true, force: true });
    await assert.rejects(access(root), { code: 'ENOENT' });
  });
  const file = join(root, 'settings.json'), state = join(root, 'state');
  const original = { model: 'synthetic-model', env: { KEEP_SYNTHETIC: 'unchanged' } };
  await mkdir(state);
  await writeFile(file, JSON.stringify(original, null, 2) + '\n', { mode: 0o600 });
  return { root, file, state, original, record: join(state, 'claude-settings.json') };
}

test('Windows canonical settings remain updateable, launchable and undoable', async context => {
  const files = await fixture(context);
  await writeClaudeSettings(files.file, files.state, target, 'win32');
  await writeClaudeSettings(files.file, files.state, next, 'win32');
  const text = await readFile(files.file, 'utf8'), saved = JSON.parse(text);
  assert.equal(saved.env.ANTHROPIC_AUTH_TOKEN, next.localToken);
  assert.equal(saved.env.ANTHROPIC_BASE_URL, next.origin + '/anthropic');
  const inspection = await inspectClaudeSettings(async path => path === files.file ? text : null,
    { platform: 'win32', env: {}, userPath: files.file, cwd: files.root, expected: saved.env });
  assert.deepEqual(inspection, { overrideBaseUrl: false, blocking: [] });
  assert.equal(await undoClaudeSettings(files.state, 'win32'), 'restored');
  assert.deepEqual(JSON.parse(await readFile(files.file, 'utf8')), files.original);
  assert.deepEqual(await readdir(files.state), []);
  assert.deepEqual((await readdir(files.root)).sort(), ['settings.json', 'state']);
});

for (const [alias, canonical] of [
  ['anthropic_auth_token', 'ANTHROPIC_AUTH_TOKEN'],
  ['AnThRoPiC_BaSe_Url', 'ANTHROPIC_BASE_URL'],
]) {
  test('Windows update preserves an externally added ' + alias, async context => {
    const files = await fixture(context);
    await writeClaudeSettings(files.file, files.state, target, 'win32');
    const saved = JSON.parse(await readFile(files.file, 'utf8'));
    saved.env[alias] = saved.env[canonical];
    const text = JSON.stringify(saved, null, 2) + '\n';
    await writeFile(files.file, text);
    const record = await readFile(files.record, 'utf8');
    await assert.rejects(writeClaudeSettings(files.file, files.state, next, 'win32'), error => {
      assert.ok(error instanceof SettingsError);
      assert.ok(error.message.includes(alias));
      assert.doesNotMatch(error.message, /synthetic-original-token|synthetic-next-token/);
      return true;
    });
    assert.equal(await readFile(files.file, 'utf8'), text);
    assert.equal(await readFile(files.record, 'utf8'), record);
    assert.deepEqual(await readdir(files.state), ['claude-settings.json']);
    assert.deepEqual((await readdir(files.root)).sort(), ['settings.json', 'state']);
  });
}
