import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { build, version as esbuildVersion } from 'esbuild';

const directory = fileURLToPath(new URL('../src/compatibility/', import.meta.url));
const compiled = await build({ stdin: { contents: [
  "export { runClient } from './command.ts';",
  "export { writeClaudeSettings } from './settings.ts';",
].join('\n'), resolveDir: directory, loader: 'ts' },
bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
const bundle = compiled.outputFiles[0].contents;
const { runClient, writeClaudeSettings } = await import(
  `data:text/javascript;base64,${Buffer.from(bundle).toString('base64')}`);
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const sources = {};
for (const path of Object.keys(compiled.metafile.inputs).filter(path => path !== '<stdin>').sort()) {
  sources[path] = digest(await readFile(resolve(path)));
}
console.log(JSON.stringify({ evidence: 'synthetic source functions, not native Windows',
  runtime: process.version, esbuildVersion, sources, bundle: digest(bundle) }));
const target = { origin: 'http://127.0.0.1:8788', localToken: 'synthetic-remote-token' };
const managed = ['ANTHROPIC_BASE_URL', 'ANTHROPIC_AUTH_TOKEN'];
async function optionalFile(path) {
  try { return await readFile(path, 'utf8'); }
  catch (error) { if (error.code === 'ENOENT') return null; throw error; }
}

async function fixture(context, platform, edit) {
  const root = await mkdtemp(join(tmpdir(), 'frm-undo-case-'));
  context.after(async () => {
    await rm(root, { recursive: true, force: true });
    assert.equal(await optionalFile(join(root, 'claude', 'settings.json')), null);
  });
  const settings = join(root, 'claude', 'settings.json');
  const state = join(root, 'state');
  await mkdir(dirname(settings), { recursive: true });
  await writeFile(settings, JSON.stringify({ model: 'synthetic', env: { KEEP_SYNTHETIC: 'yes' } }), { mode: 0o600 });
  await writeClaudeSettings(settings, state, target, platform);
  const document = JSON.parse(await readFile(settings, 'utf8'));
  edit(document.env);
  const before = `${JSON.stringify(document, null, 2)}\n`;
  await writeFile(settings, before);
  const stdout = [], stderr = [], effects = [];
  const forbidden = async () => { effects.push('external operation'); throw new Error('forbidden'); };
  const deps = { platform, home: root, stateDir: state, target,
    env: { CLAUDE_CONFIG_DIR: dirname(settings) }, cwd: root,
    runner: { run: forbidden, capture: forbidden }, fetch: forbidden,
    io: { stdout: value => stdout.push(value), stderr: value => stderr.push(value) } };
  return { settings, state, before, document, stdout, stderr, effects, deps };
}

for (const [key, alias, rename] of [
  ['ANTHROPIC_AUTH_TOKEN', 'anthropic_auth_token', true],
  ['ANTHROPIC_BASE_URL', 'AnThRoPiC_BaSe_Url', true],
  ['ANTHROPIC_AUTH_TOKEN', 'Anthropic_Auth_Token', false],
]) {
  test(`Windows undo preserves externally ${rename ? 'renamed' : 'added'} ${alias}`, { timeout: 5000 }, async context => {
    const files = await fixture(context, 'win32', env => {
      env[alias] = rename ? env[key] : 'synthetic-foreign-token';
      if (rename) delete env[key];
    });
    const code = await runClient(['claude', '--undo'], files.deps);
    const after = await readFile(files.settings, 'utf8');
    const record = await optionalFile(join(files.state, 'claude-settings.json'));
    const output = files.stdout.join('');
    context.diagnostic(JSON.stringify({ alias, code, changed: after !== files.before,
      saysRemoved: output.startsWith('Removed'), aliasRemains: Object.hasOwn(JSON.parse(after).env ?? {}, alias),
      recordRemoved: record === null, externalOperations: files.effects.length }));
    assert.deepEqual(files.effects, []);
    assert.equal(code, 0);
    assert.equal(files.stderr.join(''), '');
    assert.equal(output, 'Claude settings changed since Framerize wrote them; left them unchanged.\n');
    assert.equal(after, files.before, 'foreign aliases must not cause partial undo');
    assert.equal(record, null);
  });
}

test('POSIX undo preserves distinct aliases added after persistence', { timeout: 5000 }, async context => {
  const files = await fixture(context, 'linux', env => {
    env.anthropic_auth_token = 'synthetic-foreign-token';
    env.Anthropic_Base_Url = 'https://synthetic.invalid/anthropic';
  });
  assert.equal(await runClient(['claude', '--undo'], files.deps), 0);
  const expected = structuredClone(files.document);
  for (const key of managed) delete expected.env[key];
  assert.deepEqual(JSON.parse(await readFile(files.settings, 'utf8')), expected);
  assert.equal(files.stdout.join(''), 'Removed the Remote values from Claude settings.\n');
  assert.equal(files.stderr.join(''), '');
  assert.equal(await optionalFile(join(files.state, 'claude-settings.json')), null);
  assert.deepEqual(files.effects, []);
});
