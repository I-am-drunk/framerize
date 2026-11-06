import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { build, version as esbuildVersion } from 'esbuild';

const sourceDirectory = fileURLToPath(new URL('../src/compatibility/', import.meta.url));
const compiled = await build({ stdin: { contents: [
  "export { runClient } from './command.ts';",
  "export { inspectClaudeSettings } from './conflicts.ts';",
  "export { undoClaudeSettings } from './settings.ts';",
].join('\n'), resolveDir: sourceDirectory, loader: 'ts' },
bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
const bundle = compiled.outputFiles[0].contents;
const { runClient, inspectClaudeSettings, undoClaudeSettings } = await import(
  `data:text/javascript;base64,${Buffer.from(bundle).toString('base64')}`);
const digest = value => createHash('sha256').update(value).digest('hex');
const sources = {};
for (const path of Object.keys(compiled.metafile.inputs).filter(path => path !== '<stdin>').sort()) {
  sources[path] = digest(await readFile(resolve(path)));
}
console.log(JSON.stringify({ evidence: 'synthetic source functions, no native Windows or client',
  runtime: process.version, esbuildVersion, sources, bundle: digest(bundle) }));
const target = { origin: 'http://127.0.0.1:8788', localToken: 'synthetic-remote-token' };

async function optionalFile(path) {
  try { return await readFile(path, 'utf8'); }
  catch (error) { if (error.code === 'ENOENT') return null; throw error; }
}

async function fixture(context, platform, key) {
  const root = await mkdtemp(join(tmpdir(), 'frm-persist-case-'));
  context.after(async () => {
    await rm(root, { recursive: true, force: true });
    assert.equal(await optionalFile(join(root, 'claude', 'settings.json')), null);
  });
  const settingsPath = join(root, 'claude', 'settings.json');
  const stateDir = join(root, 'state');
  const original = `${JSON.stringify({ model: 'synthetic-model', env: {
    KEEP_SYNTHETIC: 'unchanged', [key]: 'synthetic-foreign-value',
  } }, null, 2)}\n`;
  await mkdir(dirname(settingsPath), { recursive: true });
  await writeFile(settingsPath, original, { mode: 0o600 });
  const stdout = [], stderr = [], effects = [];
  const forbidden = async () => { effects.push('unexpected external operation'); throw new Error('forbidden'); };
  const deps = { platform, home: root, stateDir, target, cwd: root,
    env: { CLAUDE_CONFIG_DIR: dirname(settingsPath) },
    runner: { run: forbidden, capture: forbidden }, fetch: forbidden,
    io: { stdout: text => stdout.push(text), stderr: text => stderr.push(text) } };
  return { root, settingsPath, stateDir, original, stdout, stderr, effects, deps };
}

for (const key of ['anthropic_auth_token', 'AnThRoPiC_BaSe_Url', 'ANTHROPIC_AUTH_TOKEN']) {
  test(`Windows persist refuses an existing foreign ${key}`, { timeout: 5000 }, async context => {
    const files = await fixture(context, 'win32', key);
    const code = await runClient(['claude', '--persist'], files.deps);
    const after = await readFile(files.settingsPath, 'utf8');
    const record = await optionalFile(join(files.stateDir, 'claude-settings.json'));
    const inspection = await inspectClaudeSettings(async path =>
      path === files.settingsPath ? after : null, { platform: 'win32', env: {},
      userPath: files.settingsPath, cwd: files.root, expected: {
        ANTHROPIC_BASE_URL: `${target.origin}/anthropic`, ANTHROPIC_AUTH_TOKEN: target.localToken,
      } });
    context.diagnostic(JSON.stringify({ key, exitCode: code, settingsChanged: after !== files.original,
      recordWritten: record !== null, subsequentLaunchBlocked: inspection.blocking.length > 0,
      subsequentLaunchOverride: inspection.overrideBaseUrl, externalOperations: files.effects.length }));
    assert.deepEqual(files.effects, []);
    assert.equal(code, 1, 'foreign Windows environment aliases must not be silently managed');
    assert.equal(after, files.original, 'refusal must preserve exact settings bytes');
    assert.equal(record, null, 'refusal must not create an undo record');
    assert.equal(files.stdout.join(''), '');
    assert.match(files.stderr.join(''), /Claude settings already set/);
    assert.doesNotMatch(files.stderr.join(''), /synthetic-foreign-value|synthetic-remote-token/);
  });
}

test('POSIX persist preserves distinct lowercase keys and remains undoable', { timeout: 5000 }, async context => {
  const files = await fixture(context, 'linux', 'anthropic_auth_token');
  assert.equal(await runClient(['claude', '--persist'], files.deps), 0);
  const saved = JSON.parse(await readFile(files.settingsPath, 'utf8'));
  assert.equal(saved.env.anthropic_auth_token, 'synthetic-foreign-value');
  assert.equal(saved.env.ANTHROPIC_AUTH_TOKEN, target.localToken);
  assert.equal(saved.env.ANTHROPIC_BASE_URL, `${target.origin}/anthropic`);
  assert.equal(saved.env.KEEP_SYNTHETIC, 'unchanged');
  assert.equal(saved.model, 'synthetic-model');
  assert.match(files.stdout.join(''), /Claude settings now use/);
  assert.equal(files.stderr.join(''), '');
  assert.equal(await undoClaudeSettings(files.stateDir, files.deps.platform), 'restored');
  assert.deepEqual(JSON.parse(await readFile(files.settingsPath, 'utf8')), JSON.parse(files.original));
  assert.equal(await optionalFile(join(files.stateDir, 'claude-settings.json')), null);
  assert.deepEqual(files.effects, []);
});
