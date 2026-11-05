import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { compareSemverVersions } from './semver.ts'; export { versionStatus } from './versions.ts'; export { checkVersion } from './latest.ts';",
  resolveDir: fileURLToPath(new URL('../src/compatibility/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false });
const bytes = compiled.outputFiles[0].contents;
const bundleSha256 = createHash('sha256').update(bytes).digest('hex');
const { compareSemverVersions: compare, versionStatus, checkVersion } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

for (const [field, older, newer] of [
  ['major', '9007199254740992.0.0', '9007199254740993.0.0'],
  ['minor', '1.9007199254740992.0', '1.9007199254740993.0'],
  ['patch', '1.0.9007199254740992', '1.0.9007199254740993'],
  ['prerelease', '1.0.0-rc.9007199254740992', '1.0.0-rc.9007199254740993'],
]) {
  test(`large numeric ${field} identifiers retain their ordering`, context => {
    context.diagnostic(`source bundle sha256 ${bundleSha256}`);
    assert.ok(compare(older, newer) < 0, 'newer identifier must sort after the older identifier');
    assert.ok(compare(newer, older) > 0, 'reverse ordering must agree');
    assert.equal(compare(older, older), 0);
    assert.equal(versionStatus(older, newer, compare).status, 'behind');
    assert.equal(versionStatus(newer, older, compare).status, 'current');
  });
}

test('numeric prereleases larger than Number remain finite and reflexive', () => {
  const older = `1.0.0-${'9'.repeat(400)}`;
  const newer = `1.0.0-1${'0'.repeat(400)}`;
  assert.ok(Number.isFinite(compare(older, newer)));
  assert.ok(compare(older, newer) < 0);
  assert.ok(compare(newer, older) > 0);
  assert.equal(compare(older, older), 0);
});

test('numeric ordering keeps stable, ASCII and build-metadata behavior', () => {
  const ordered = ['1.0.0-2', '1.0.0-11', '1.0.0-Z', '1.0.0-a', '1.0.0', '1.0.1'];
  for (const [index, current] of ordered.entries()) {
    assert.equal(compare(current, current), 0);
    if (index) assert.ok(compare(ordered[index - 1], current) < 0);
  }
  assert.equal(compare('1.0.0+left', '1.0.0+right'), 0);
  assert.equal(compare('v1.2', '1.2.0'), 0);
  assert.ok(compare('01.2.0', '1.3.0') < 0);
});

test('synthetic npm version lookup reports a later large prerelease', { timeout: 2000 }, async context => {
  const older = '1.0.0-rc.9007199254740992';
  const newer = '1.0.0-rc.9007199254740993';
  const captured = [];
  const result = await checkVersion('claude', '/synthetic/bin/claude', {
    kind: 'manual', reason: 'synthetic fixture', owner: { kind: 'npm', prefix: '/synthetic' },
  }, {
    platform: 'linux', env: { PATH: '/synthetic/bin' }, home: '/synthetic/home',
    probe: path => path === '/synthetic/bin/npm',
    read: async () => assert.fail('no settings read is permitted'),
    fetch: async () => assert.fail('no network fetch is permitted'),
    runner: {
      run: async () => assert.fail('no updater or client launch is permitted'),
      capture: async command => {
        captured.push([command.command, command.args]);
        return command.command === '/synthetic/bin/claude'
          ? `${older} (synthetic Claude)` : JSON.stringify(newer);
      },
    },
  });
  context.diagnostic(JSON.stringify(result));
  assert.deepEqual(captured, [
    ['/synthetic/bin/claude', ['--version']],
    ['/synthetic/bin/npm', ['view', '--global', '--prefix', '/synthetic', '--json', '@anthropic-ai/claude-code@latest', 'version']],
  ]);
  assert.deepEqual(result, { status: 'behind', current: older, latest: newer });
});
