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
  writeFileSync(path.join(directory, 'dist', 'SHA256SUMS'), sums);
  return spawnSync(process.execPath, ['scripts/release-metadata.js'], {
    encoding: 'utf8', env: { ...process.env, RELEASE_WORKSPACE: directory,
      GITHUB_REF_NAME: '', GITHUB_SHA: '', ...env },
  });
}

test('release metadata accepts a matching version tag', () => {
  const result = metadata({ GITHUB_REF_NAME: 'framer-next-v0.1.5' });
  assert.equal(result.status, 0, result.stderr);
});

test('release metadata rejects empty and duplicate checksum entries', () => {
  assert.notEqual(metadata({}, '').status, 0);
  const line = `${'0'.repeat(64)}  artifact.tgz\n`;
  assert.match(metadata({}, line + line).stderr, /duplicate artifact/);
});

test('release metadata rejects malformed package versions', () => {
  assert.match(metadata({}, undefined, 'v0.1.5').stderr, /valid SemVer/);
  const result = metadata({ GITHUB_REF_NAME: 'framer-next-vnot-semver' });
  assert.notEqual(result.status, 0);
  assert.notEqual(metadata({ GITHUB_REF_NAME: 'framer-next-v0.1.5-01' }).status, 0);
});

test('release metadata rejects a mismatched version tag', () => {
  const result = metadata({ GITHUB_REF_NAME: 'framer-next-v9.9.9' });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /does not match package version/);
});

test('release metadata rejects a changed source commit', () => {
  const result = metadata({ GITHUB_SHA: '0'.repeat(40) });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /source commit/);
});
