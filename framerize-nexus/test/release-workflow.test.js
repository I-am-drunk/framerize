import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const workflow = readFileSync(new URL('../../.github/workflows/release.yml', import.meta.url), 'utf8');

test('release publishes binaries before attempting npm authentication', () => {
  const release = workflow.indexOf('name: Publish GitHub release');
  const npm = workflow.indexOf('name: Publish npm package');
  assert.ok(release > 0);
  assert.ok(npm > release);
});

test('release verifies the packaged artifacts before upload', () => {
  const upload = workflow.indexOf('uses: actions/upload-artifact@');
  assert.ok(upload > 0);
  assert.match(workflow, /macos-15/);
  assert.match(workflow, /windows-2022/);
  assert.match(workflow, /framerize-nexus-macos-arm64\.tar\.gz/);
  assert.match(workflow, /framerize-nexus-windows-x64\.zip/);
  assert.match(workflow, /dist\/framerize-nexus\.exe/);
  assert.match(workflow, /node scripts\/merge-release-manifests\.js/);
  assert.match(workflow, /npm publish \.\/dist\/framerize-nexus-\*\.tgz/);
  assert.match(workflow, /working-directory: framerize-nexus/);
});

test('CI runs npm commands from the connector directory', () => {
  const ci = readFileSync(new URL('../../.github/workflows/ci.yml', import.meta.url), 'utf8');
  assert.match(ci, /working-directory: framerize-nexus/);
  assert.match(ci, /cache-dependency-path: framerize-nexus\/package-lock\.json/);
});
