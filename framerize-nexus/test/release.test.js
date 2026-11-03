import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

function metadata(env = {}, sums = `${'0'.repeat(64)}  artifact.tgz\n`, version = '0.1.5') {
  const directory = mkdtempSync(path.join(tmpdir(), 'framer-next-release-'));
  mkdirSync(path.join(directory, 'dist'));
  writeFileSync(path.join(directory, 'package.json'), JSON.stringify({ version }));
  writeFileSync(path.join(directory, `framerize-nexus-${version}.tgz`), 'package');
  writeFileSync(path.join(directory, 'dist', 'SHA256SUMS'), sums);
  writeFileSync(path.join(directory, 'dist', 'LICENSE'), 'project license');
  writeFileSync(path.join(directory, 'dist', 'NODE-LICENSE'), 'node license');
  return spawnSync(process.execPath, ['scripts/release-metadata.js'], {
    encoding: 'utf8', env: { ...process.env, RELEASE_WORKSPACE: directory,
      GITHUB_REF_NAME: '', GITHUB_REF_TYPE: '', GITHUB_SHA: '', ...env },
  });
}

test('release metadata accepts a matching version tag', () => {
  const result = metadata({ GITHUB_REF_NAME: 'framerize-nexus-v0.1.5', GITHUB_REF_TYPE: 'tag' });
  assert.equal(result.status, 0, result.stderr);
});

test('release metadata rejects empty and duplicate checksum entries', () => {
  assert.notEqual(metadata({}, '').status, 0);
  const line = `${'0'.repeat(64)}  artifact.tgz\n`;
  assert.match(metadata({}, line + line).stderr, /duplicate artifact/);
});

test('release metadata rejects malformed package versions', () => {
  assert.match(metadata({}, undefined, 'v0.1.5').stderr, /valid SemVer/);
  const result = metadata({ GITHUB_REF_NAME: 'framerize-nexus-vnot-semver', GITHUB_REF_TYPE: 'tag' });
  assert.notEqual(result.status, 0);
  assert.notEqual(metadata({ GITHUB_REF_NAME: 'framerize-nexus-v0.1.5-01', GITHUB_REF_TYPE: 'tag' }).status, 0);
});

test('release metadata rejects a mismatched version tag', () => {
  const result = metadata({ GITHUB_REF_NAME: 'framerize-nexus-v9.9.9', GITHUB_REF_TYPE: 'tag' });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /does not match package version/);
});

test('release metadata rejects a changed source commit', () => {
  const result = metadata({ GITHUB_SHA: '0'.repeat(40) });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /source commit/);
});

test('release metadata permits a branch rehearsal', () => {
  const result = metadata({ GITHUB_REF_NAME: 't3/nexus-rehearsal', GITHUB_REF_TYPE: 'branch' });
  assert.equal(result.status, 0, result.stderr);
});
