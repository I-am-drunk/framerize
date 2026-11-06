import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/latest.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { parseChocolateyVersion } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

test('Chocolatey preserves comparable release and prerelease identifiers', () => {
  for (const version of ['0.0.0', '2.1.1', '2.1.1-alpha-1', '2.1.1-1',
    '2.1.1+build.5', '2.1.1-rc.1+build.5']) {
    assert.equal(parseChocolateyVersion(`noise\r\nCLAUDE-CODE| ${version} \r\n`, 'claude-code'), version);
  }
});

test('Chocolatey does not compare malformed or unsupported release identifiers', () => {
  for (const version of ['2.1.1+', '2.1.1-01', '2.1.1-..', '2.1.1+build..5',
    'v2.1.1', '2.1', '2.1.1.4', '2.1.1 installed', '02.1.1']) {
    assert.equal(parseChocolateyVersion(`claude-code|${version}`, 'claude-code'), null, version);
  }
  assert.equal(parseChocolateyVersion(null, 'claude-code'), null);
});
