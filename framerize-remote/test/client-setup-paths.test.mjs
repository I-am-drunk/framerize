// Path classification cases adapted from T3 Code
// packages/provider-core/src/server/maintenanceResolver.test.ts at 5a906583 (MIT).
import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/compatibility/index.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
const setup = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const plan = (client, resolvedPath, realPath = resolvedPath, platform = 'linux', env = {}) =>
  setup.planUpdate(client, { resolvedPath, realPath, platform, env });

test('npm prefix comes only from the global lib/node_modules layout', () => {
  const { npmGlobalPrefixFromCommandPath: prefix } = setup;
  assert.equal(prefix('/usr/local/lib/node_modules/@openai/codex/bin/codex.js', '@openai/codex'), '/usr/local');
  assert.equal(prefix('/usr/local/lib/node_modules/other/node_modules/@openai/codex/bin/codex.js', '@openai/codex'), null);
  assert.equal(prefix('/lib/node_modules/@openai/codex/bin/codex.js', '@openai/codex'), '/');
  assert.equal(prefix('/work/app/node_modules/@openai/codex/bin/codex.js', '@openai/codex'), null);
});

test('Codex standalone home keeps root separators', () => {
  assert.equal(setup.codexStandaloneHome('/home/me/.codex/packages/standalone/current/bin/codex'), '/home/me/.codex');
  assert.equal(setup.codexStandaloneHome('C:\\Users\\me\\.codex\\packages\\standalone\\bin\\codex.exe'), 'C:\\Users\\me\\.codex');
  assert.equal(setup.codexStandaloneHome('/packages/standalone/bin/codex'), '/');
  assert.equal(setup.codexStandaloneHome('/usr/local/bin/codex'), null);
});

const winGetLink = 'C:\\Users\\me\\AppData\\Local\\Microsoft\\WinGet\\Links\\claude.exe';
const winGetPackage = 'C:\\Users\\me\\AppData\\Local\\Microsoft\\WinGet\\Packages\\Anthropic.ClaudeCode_Microsoft.Winget.Source_8wekyb3d8bbwe\\claude.exe';
const nativeWindowsClaude = 'C:\\Users\\me\\.local\\bin\\claude.exe';

for (const [name, resolved, real] of [['a user link', winGetLink, winGetPackage],
  ['a mixed-case link', winGetLink.toUpperCase(), winGetPackage], ['a native-looking alias', nativeWindowsClaude, winGetPackage]]) {
  test(`a WinGet Claude install stays manual through ${name}`, () => {
    assert.deepEqual(plan('claude', resolved, real, 'win32'),
      { kind: 'manual', reason: 'winget upgrade Anthropic.ClaudeCode', owner: { kind: 'winget' } });
  });
}

test('a Chocolatey Claude install stays manual, but not off Windows', () => {
  const shim = 'C:\\ProgramData\\chocolatey\\bin\\claude.exe';
  assert.deepEqual(plan('claude', shim, shim, 'win32'),
    { kind: 'manual', reason: 'choco upgrade claude-code', owner: { kind: 'chocolatey' } });
  assert.equal(plan('claude', '/home/me/chocolatey/bin/claude.exe', undefined, 'linux').kind, 'manual');
  assert.notEqual(plan('claude', '/home/me/chocolatey/bin/claude.exe').reason, 'choco upgrade claude-code');
});

test('a native launcher that delegates to npm or mise is not updated natively', () => {
  const npm = plan('claude', '/home/me/.local/bin/claude', '/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js');
  assert.equal(npm.command.command, 'npm');
  assert.equal(plan('claude', '/home/me/.local/bin/claude', '/home/me/.local/share/mise/installs/claude/2.1/claude').kind, 'manual');
});

test('Codex standalone updates carry CODEX_HOME and bun globals use bun', () => {
  const standalone = plan('codex', '/home/me/.codex/packages/standalone/current/bin/codex');
  assert.deepEqual(standalone.command.args, ['update']);
  assert.deepEqual(standalone.command.env, { CODEX_HOME: '/home/me/.codex' });
  assert.deepEqual(plan('codex', '/home/me/.bun/bin/codex').command.args, ['i', '-g', '@openai/codex@latest']);
  assert.equal(plan('codex', '/work/app/node_modules/.bin/codex').kind, 'manual');
});
