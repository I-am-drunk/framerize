import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

function installStub(bin, name) {
  const file = path.join(bin, name);
  const dispatcher = fileURLToPath(new URL('fixtures/command-fixture.js', import.meta.url));
  const script = `#!/usr/bin/env node\nprocess.argv.splice(2, 0, ${JSON.stringify(name)});\nawait import(${JSON.stringify(dispatcher)});`;
  writeFileSync(file, script, { mode: 0o755 });
  if (process.platform === 'win32') {
    const quoted = process.execPath.replace(/"/g, '\\"');
    for (const extension of ['bat']) writeFileSync(path.join(bin, `${name}.${extension}`),
      `@echo off\r\n"${quoted}" "${dispatcher}" "${name}" %*\r\n`);
  }
}

function publish(authenticated = true, published = false, version = '0.1.7',
  registryError = '', githubActions = false, checksumValid = true, commit = 'a'.repeat(40),
  includeDigest = true, dirty = false) {
  const directory = mkdtempSync(path.join(tmpdir(), 'framer-next-publish-'));
  const bin = path.join(directory, 'bin');
  mkdirSync(bin);
  mkdirSync(path.join(directory, 'dist'));
  writeFileSync(path.join(directory, 'dist', `framer-next-${version}.tgz`), 'fixture');
  writeFileSync(path.join(directory, 'dist', 'SHA256SUMS'),
    `${'0'.repeat(64)}  framer-next\n${'0'.repeat(64)}  framer-next-${version}.tgz\n`);
  writeFileSync(path.join(directory, 'dist', 'release.json'), JSON.stringify({
    version, commit,
    artifacts: includeDigest ? { [`framer-next-${version}.tgz`]: '0'.repeat(64) } : {},
  }));
  const calls = path.join(directory, 'calls');
  const fixture = JSON.stringify({ authenticated, published, version,
    registryError, checksumValid, commit: `${commit}^{commit}`, dirty, calls });
  for (const name of ['npm', 'sha256sum', 'git', 'tar']) installStub(bin, name);
  const result = spawnSync(process.execPath, [
    fileURLToPath(new URL('../scripts/publish-package.js', import.meta.url))], {
    encoding: 'utf8',
    cwd: directory,
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}`,
      GITHUB_ACTIONS: githubActions ? 'true' : '',
      FRAMER_NEXT_TEST_COMMAND: fixture },
  });
  return { result, calls, fixture };
}

function publishCommit(recordedCommit) {
  const fixture = publish();
  assert.equal(fixture.result.status, 0, fixture.result.stderr);
  const directory = path.dirname(fixture.calls);
  const metadata = JSON.parse(readFileSync(path.join(directory, 'dist/release.json'), 'utf8'));
  metadata.commit = recordedCommit;
  writeFileSync(path.join(directory, 'dist/release.json'), JSON.stringify(metadata));
  return spawnSync(process.execPath, [fileURLToPath(new URL('../scripts/publish-package.js', import.meta.url))], {
    encoding: 'utf8', cwd: directory,
    env: { ...process.env, PATH: `${path.join(directory, 'bin')}:${process.env.PATH}`,
      FRAMER_NEXT_TEST_COMMAND: fixture.fixture },
  });
}

test('a dirty checkout prevents publication', () => {
  const { result } = publish(true, false, '0.1.7', '', false, true, 'a'.repeat(40), true, true);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /clean source checkout/);
});

test('release metadata must include the tarball digest', () => {
  const { result } = publish(true, false, '0.1.7', '', false, true, 'a'.repeat(40), false);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /does not contain the tarball SHA-256/);
});

test('tarball metadata must match the release filename', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'framer-next-metadata-'));
  mkdirSync(path.join(directory, 'dist'));
  mkdirSync(path.join(directory, 'package'));
  writeFileSync(path.join(directory, 'package', 'package.json'),
    JSON.stringify({ name: 'framer-next', version: '0.1.8' }));
  spawnSync('tar', ['-czf', 'dist/framer-next-0.1.7.tgz', 'package'], { cwd: directory });
  writeFileSync(path.join(directory, 'dist', 'release.json'),
    JSON.stringify({ version: '0.1.7', commit: 'a'.repeat(40) }));
  const result = spawnSync(process.execPath, [
    fileURLToPath(new URL('../scripts/publish-package.js', import.meta.url))], {
    cwd: directory, encoding: 'utf8',
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /tarball package metadata/);
});

test('authenticated publication verifies the package before publishing', () => {
  const { result, calls } = publish();
  assert.equal(result.status, 0, result.stderr);
  const commands = readFileSync(calls, 'utf8').trim().split('\n');
  assert.match(commands[2], /^whoami /);
  assert.match(commands[3], /^view /);
  assert.match(commands[4], /^-c .*SHA256SUMS$/);
  assert.match(commands[5], /^publish .*--tag latest$/);
});

test('checksum filtering excludes only missing release files', () => {
  const { result, calls } = publish();
  assert.equal(result.status, 0, result.stderr);
  const checksum = readFileSync(path.join(path.dirname(calls),
    'dist/.publish-SHA256SUMS'), 'utf8');
  assert.doesNotMatch(checksum, /framer-next\n/);
  assert.match(checksum, /framer-next-0.1.7.tgz/);
});

test('provenance is enabled only in GitHub Actions', () => {
  const local = publish();
  assert.doesNotMatch(readFileSync(local.calls, 'utf8'), /--provenance/);
  const ci = publish(true, false, '0.1.7', '', true);
  assert.match(readFileSync(ci.calls, 'utf8'), /^publish .*--provenance/m);
});

test('an existing version is never republished', () => {
  const { result, calls } = publish(true, true);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /already published/);
  assert.doesNotMatch(readFileSync(calls, 'utf8'), /^publish /m);
});

test('prereleases use the next dist-tag', () => {
  const { result, calls } = publish(true, false, '0.2.0-rc.1');
  assert.equal(result.status, 0, result.stderr);
  assert.match(readFileSync(calls, 'utf8'), /--tag next$/m);
});

test('registry failures prevent packaging and publication', () => {
  const { result, calls } = publish(true, false, '0.1.7', 'E503 unavailable');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /cannot verify/);
  assert.equal(readFileSync(calls, 'utf8').trim().split('\n').length, 4);
});

test('checksum failures prevent publication', () => {
  const { result, calls } = publish(true, false, '0.1.7', '', false, false);
  assert.notEqual(result.status, 0);
  assert.doesNotMatch(readFileSync(calls, 'utf8'), /^publish /m);
});

test('unauthenticated publication makes no package changes', () => {
  const { result, calls } = publish(false);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /requires authenticated package-owner credentials/);
  assert.equal(readFileSync(calls, 'utf8').trim().split('\n').length, 3);
});

test('ambiguous release artifacts are rejected', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'framer-next-artifacts-'));
  mkdirSync(path.join(directory, 'dist'));
  for (const version of ['0.1.7', '0.1.8']) {
    writeFileSync(path.join(directory, 'dist', `framer-next-${version}.tgz`), 'fixture');
  }
  const result = spawnSync(process.execPath, [
    fileURLToPath(new URL('../scripts/publish-package.js', import.meta.url))], {
    cwd: directory, encoding: 'utf8',
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /exactly one Framer Next tarball/);
});

test('a different source commit prevents publication', () => {
  const result = publishCommit('b'.repeat(40));
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /not present in this repository/);
});

test('malformed release source metadata prevents publication', () => {
  const result = publishCommit('not-a-commit');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /invalid version or source commit/);
});

test('malformed checksum entries prevent publication', () => {
  const fixture = publish();
  assert.equal(fixture.result.status, 0, fixture.result.stderr);
  const directory = path.dirname(fixture.calls);
  writeFileSync(path.join(directory, 'dist/SHA256SUMS'), 'not-a-checksum\n');
  const result = spawnSync(process.execPath, [
    fileURLToPath(new URL('../scripts/publish-package.js', import.meta.url))], {
    encoding: 'utf8', cwd: directory,
    env: { ...process.env, PATH: `${path.join(directory, 'bin')}:${process.env.PATH}`,
      FRAMER_NEXT_TEST_COMMAND: fixture.fixture },
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /invalid SHA256SUMS entry/);
});

test('conflicting release metadata prevents publication', () => {
  const fixture = publish();
  assert.equal(fixture.result.status, 0, fixture.result.stderr);
  const directory = path.dirname(fixture.calls);
  const metadata = JSON.parse(readFileSync(path.join(directory, 'dist/release.json'), 'utf8'));
  metadata.artifacts['framer-next-0.1.7.tgz'] = '1'.repeat(64);
  writeFileSync(path.join(directory, 'dist/release.json'), JSON.stringify(metadata));
  const result = spawnSync(process.execPath, [
    fileURLToPath(new URL('../scripts/publish-package.js', import.meta.url))], {
    encoding: 'utf8', cwd: directory,
    env: { ...process.env, PATH: `${path.join(directory, 'bin')}:${process.env.PATH}`,
      FRAMER_NEXT_TEST_COMMAND: fixture.fixture },
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /digests differ/);
  assert.equal((readFileSync(fixture.calls, 'utf8').match(/^publish /gm) || []).length, 1);
});

test('publishing a prior release verifies its recorded source commit', () => {
  const { result, calls } = publish(true, false, '0.1.7', '', false, true, 'c'.repeat(40));
  assert.equal(result.status, 0, result.stderr);
  assert.match(readFileSync(calls, 'utf8'), /^cat-file -e c{40}\^\{commit\}$/m);
  assert.match(readFileSync(calls, 'utf8'), /^publish .*--tag latest$/m);
});
