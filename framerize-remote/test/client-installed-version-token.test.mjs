import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { parseInstalledVersion } from './versions.ts'; export { runClient } from './command.ts';",
  resolveDir: fileURLToPath(new URL('../src/compatibility/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false });
const bytes = compiled.outputFiles[0].contents;
const bundleSha256 = createHash('sha256').update(bytes).digest('hex');
const { parseInstalledVersion, runClient } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

for (const version of ['1.2.3-alpha-', '1.2.3--', '1.2.3-alpha.-']) {
  test(`installed version retains the entire ${version} prerelease`, context => {
    context.diagnostic(`source bundle sha256 ${bundleSha256}`);
    assert.equal(parseInstalledVersion(`${version} (synthetic Claude)\n`), version);
    assert.equal(parseInstalledVersion(`synthetic-cli ${version}\n`), version);
  });
}

test('ordinary installed-version output remains supported', () => {
  assert.equal(parseInstalledVersion('1.2.3 (synthetic Claude)\n'), '1.2.3');
  assert.equal(parseInstalledVersion('synthetic-cli 1.2.3-beta.2\n'), '1.2.3-beta.2');
  assert.equal(parseInstalledVersion('no version'), null);
  assert.equal(parseInstalledVersion(null), null);
});

test('a stable update is not suppressed for a hyphen-only prerelease', { timeout: 2000 }, async context => {
  const executable = '/synthetic/home/.local/bin/claude';
  const captures = [], runs = [], fetches = [], reads = [], output = [], errors = [];
  const result = await runClient(['claude', '--update', '--no-input'], {
    platform: 'linux', env: { PATH: '/synthetic/home/.local/bin' }, home: '/synthetic/home',
    stateDir: '/synthetic/state', target: { origin: 'http://127.0.0.1:8788' },
    probe: path => path === executable, realPath: path => path,
    io: { stdout: text => output.push(text), stderr: text => errors.push(text) },
    read: async path => { reads.push(path); return null; },
    fetch: async url => { fetches.push(url); return '1.2.3'; },
    runner: {
      capture: async command => { captures.push(command); return '1.2.3-- (synthetic Claude)\n'; },
      run: async command => { runs.push(command); return 0; },
    },
  });
  context.diagnostic(JSON.stringify({ output, errors, captures, runs, fetches, reads }));
  assert.equal(result, 0);
  assert.deepEqual(errors, []);
  assert.deepEqual(captures.map(command => [command.command, command.args]), [[executable, ['--version']]]);
  assert.deepEqual(fetches, ['https://downloads.claude.ai/claude-code-releases/latest']);
  assert.deepEqual(reads, ['/synthetic/home/.claude/settings.json', '/etc/claude-code/managed-settings.json']);
  assert.deepEqual(runs.map(command => [command.command, command.args]), [[executable, ['update']]]);
  assert.match(output.join(''), /Version 1\.2\.3--; 1\.2\.3 is available\./);
  assert.doesNotMatch(output.join(''), /is the latest/);
});
