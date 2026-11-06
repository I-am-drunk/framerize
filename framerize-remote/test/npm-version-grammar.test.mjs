import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const entry = fileURLToPath(new URL('../src/compatibility/versions.ts', import.meta.url));
const bundle = await build({ entryPoints: [entry], bundle: true, write: false,
  platform: 'node', format: 'esm' });
const { parseNpmViewVersion } = await import(`data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].contents).toString('base64')}`);

test('npm versions preserve valid prerelease and build identifiers', () => {
  for (const version of ['0.0.0', '12.34.56', '1.2.3-0', '1.2.3-alpha.0',
    '1.2.3-01a.x-y', '1.2.3+000.sha-a', '1.2.3-rc.2+build.005']) {
    assert.equal(parseNpmViewVersion(JSON.stringify(` ${version}\n`)), version);
    assert.equal(parseNpmViewVersion(JSON.stringify([version])), version);
  }
});

test('npm versions reject incomplete, noncanonical and malformed identifiers', () => {
  for (const version of ['1.2', 'v1.2.3', '01.2.3', '1.02.3', '1.2.03',
    '1.2.3-01', '1.2.3-', '1.2.3-a..b', '1.2.3+build..sha', '1.2.3_rc',
    '1.2.3+build_', '1.2.3 extra', '1.2.3\n9.9.9']) {
    assert.equal(parseNpmViewVersion(JSON.stringify(version)), null, version);
  }
});
