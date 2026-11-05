import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { compareSemverVersions } from './semver.ts'; export { versionStatus } from './versions.ts';",
  resolveDir: fileURLToPath(new URL('../src/compatibility/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false });
const { compareSemverVersions, versionStatus } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

test('ASCII-later prereleases are offered as updates', () => {
  assert.equal(versionStatus('1.0.0-Z', '1.0.0-a', compareSemverVersions).status, 'behind');
  assert.ok(compareSemverVersions('1.0.0-Z', '1.0.0-a') < 0);
});

test('ASCII-earlier prereleases are not offered as updates', () => {
  assert.equal(versionStatus('1.0.0-a', '1.0.0-Z', compareSemverVersions).status, 'current');
  assert.ok(compareSemverVersions('1.0.0-a', '1.0.0-Z') > 0);
});

test('ordinary numeric, prerelease and stable ordering remains intact', () => {
  const ordered = ['1.0.0-alpha', '1.0.0-alpha.1', '1.0.0-alpha.beta',
    '1.0.0-beta', '1.0.0-beta.2', '1.0.0-beta.11', '1.0.0-rc.1', '1.0.0', '1.0.1'];
  for (const [index, version] of ordered.entries()) {
    assert.equal(compareSemverVersions(version, version), 0);
    const next = ordered[index + 1];
    if (next) assert.ok(compareSemverVersions(version, next) < 0);
  }
  assert.equal(compareSemverVersions('1.0.0+local', '1.0.0+other'), 0);
});
