import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const workflow = readFileSync(new URL('../../.github/workflows/release.yml', import.meta.url), 'utf8');

test('release rehearses assembly without publication privileges', () => {
  assert.doesNotMatch(workflow, /npm publish|action-gh-release|id-token:|contents: write/);
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
  assert.match(workflow, /cd dist && sha256sum -c SHA256SUMS/);
  assert.match(workflow, /working-directory: framerize-nexus/);
});

test('CI tests one package tarball on every release platform', () => {
  const ci = readFileSync(new URL('../../.github/workflows/ci.yml', import.meta.url), 'utf8');
  assert.match(ci, /working-directory: framerize-nexus/);
  assert.match(ci, /cache-dependency-path: framerize-nexus\/package-lock\.json/);
  assert.match(ci, /name: framerize-nexus-package/);
  assert.match(ci, /macos-15/);
  assert.match(ci, /windows-2022/);
});

test('the macOS platform job runs fixture cleanup before building', () => {
  const ci = readFileSync(new URL('../../.github/workflows/ci.yml', import.meta.url), 'utf8');
  const platformJob = ci.match(/^  platforms:\r?\n([\s\S]*)/m)?.[1];
  assert.ok(platformJob);
  assert.match(platformJob, /os: macos-15/);
  assert.match(platformJob, /name: Verify fixture cleanup on macOS\r?\n        if: runner\.os == 'macOS'\r?\n        run: node --test test\/smoke-cleanup\.test\.js/);
  assert.ok(platformJob.indexOf('node --test test/smoke-cleanup.test.js')
    < platformJob.indexOf('npm run binary'));
});
