import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

function publish(authenticated = true, published = false, version = '0.1.7',
  registryError = '', githubActions = false, checksumValid = true, commit = 'a'.repeat(40),
  includeDigest = true, dirty = false) {
  const directory = mkdtempSync(path.join(tmpdir(), 'framer-next-publish-'));
  const bin = path.join(directory, 'bin');
  mkdirSync(bin);
  mkdirSync(path.join(directory, 'dist'));
  writeFileSync(path.join(directory, 'dist', `framer-next-${version}.tgz`), 'fixture');
  writeFileSync(path.join(directory, 'dist', 'release.json'), JSON.stringify({
    version, commit,
    artifacts: includeDigest ? { [`framer-next-${version}.tgz`]: '0'.repeat(64) } : {},
  }));
  const calls = path.join(directory, 'calls');
  const stub = `#!/bin/sh\nprintf '%s\\n' "$*" >> '${calls}'\n${
    authenticated ? '' : '[ "$1" = whoami ] && exit 1\n'}${
    published ? '[ "$1" = view ] && exit 0\n'
      : `[ "$1" = view ] && { printf '%s' '${registryError || 'E404'}' >&2; exit 1; }\n`}exit 0\n`;
  writeFileSync(path.join(bin, 'npm'), stub, { mode: 0o755 });
  writeFileSync(path.join(bin, 'sha256sum'),
    `#!/bin/sh\nprintf '%s\\n' "$*" >> '${calls}'\n${checksumValid ? '' : 'exit 1\n'}`);
  chmodSync(path.join(bin, 'sha256sum'), 0o755);
  writeFileSync(path.join(bin, 'git'),
    `#!/bin/sh\n[ "$1" = rev-parse ] && printf "%s\\n" "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"\n${
      dirty ? '[ "$1" = status ] && printf " M source.js\\n"\n' : ''}exit 0\n`,
    { mode: 0o755 });
  writeFileSync(path.join(bin, 'tar'), `#!/bin/sh\nprintf '%s\\n' '{"name":"framer-next","version":"${version}"}'\n`,
    { mode: 0o755 });
  const result = spawnSync(process.execPath, [
    new URL('../scripts/publish-package.js', import.meta.url).pathname], {
    encoding: 'utf8',
    cwd: directory,
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}`,
      GITHUB_ACTIONS: githubActions ? 'true' : '' },
  });
  return { result, calls };
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
    new URL('../scripts/publish-package.js', import.meta.url).pathname], {
    cwd: directory, encoding: 'utf8',
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /tarball package metadata/);
});

test('authenticated publication verifies the package before publishing', () => {
  const { result, calls } = publish();
  assert.equal(result.status, 0, result.stderr);
  const commands = readFileSync(calls, 'utf8').trim().split('\n');
  assert.match(commands[0], /^whoami /);
  assert.match(commands[1], /^view /);
  assert.match(commands[2], /^-c .*SHA256SUMS$/);
  assert.match(commands[3], /^publish .*--tag latest$/);
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
  assert.equal(readFileSync(calls, 'utf8').trim().split('\n').length, 2);
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
  assert.equal(readFileSync(calls, 'utf8').trim().split('\n').length, 1);
});

test('ambiguous release artifacts are rejected', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'framer-next-artifacts-'));
  mkdirSync(path.join(directory, 'dist'));
  for (const version of ['0.1.7', '0.1.8']) {
    writeFileSync(path.join(directory, 'dist', `framer-next-${version}.tgz`), 'fixture');
  }
  const result = spawnSync(process.execPath, [
    new URL('../scripts/publish-package.js', import.meta.url).pathname], {
    cwd: directory, encoding: 'utf8',
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /exactly one Framer Next tarball/);
});

test('a different source commit prevents publication', () => {
  const { result } = publish(true, false, '0.1.7', '', false, true, 'b'.repeat(40));
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /do not match the checkout/);
});
