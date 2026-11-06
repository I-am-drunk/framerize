import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const bundle = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/versions.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { parseInstalledVersion } = await import(
  `data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].contents).toString('base64')}`);

test('build metadata and punctuation preserve complete prerelease identifiers', () => {
  for (const version of ['1.2.3', '1.2.3-alpha-', '1.2.3--', '1.2.3-alpha.-']) {
    assert.equal(parseInstalledVersion(`client ${version}+build.7\n`), version);
    assert.equal(parseInstalledVersion(`client (${version})\n`), version);
  }
});

test('adjacent malformed text cannot shorten an installed version', () => {
  for (const output of ['1.2.3garbage', '1.2.3_', '1.2.3-', '1.2.3-alpha_bad']) {
    assert.equal(parseInstalledVersion(`client ${output}\n`), null, output);
  }
});

test('a malformed first candidate cannot select a later version', () => {
  for (const output of ['Version 1.2.3. Node 22.23.4', 'tool 1.2.3- node 22.23.4',
    '1.2.3.4', 'client 1.2.3garbage runtime 22.23.4', 'client 1.2.3-alpha_bad node 22.23.4']) {
    assert.equal(parseInstalledVersion(output), null, output);
  }
  assert.equal(parseInstalledVersion('codex-cli 0.170.0\r\nNode 22.23.4'), '0.170.0');
  assert.equal(parseInstalledVersion('1.2.3-- (Claude Code)\r\nNode 22.23.4'), '1.2.3--');
});
