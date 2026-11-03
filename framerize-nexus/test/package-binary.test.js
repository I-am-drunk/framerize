import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const executableName = process.platform === 'win32' ? 'framerize-nexus.exe' : 'framerize-nexus';
const archiveName = `framerize-nexus-${{ darwin: 'macos', win32: 'windows' }[process.platform] || process.platform}-${process.arch}.${process.platform === 'win32' ? 'zip' : 'tar.gz'}`;

function packageFixture(directory, nodeLicense) {
  mkdirSync(path.join(directory, 'dist'));
  writeFileSync(path.join(directory, 'dist', executableName), 'fixture');
  const result = spawnSync(process.execPath, [
    fileURLToPath(new URL('../scripts/package-binary.js', import.meta.url))], {
    cwd: directory, encoding: 'utf8',
    env: { ...process.env, NODE_DISTRIBUTION_LICENSE: nodeLicense },
  });
  assert.equal(result.status, 0, result.stderr);
}

test('binary packaging preserves existing checksums and adds the archive', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'framer-next-package-'));
  mkdirSync(path.join(directory, 'dist'));
  const executable = process.platform === 'win32' ? 'framerize-nexus.exe' : 'framerize-nexus';
  writeFileSync(path.join(directory, 'dist', executable), 'fixture');
  writeFileSync(path.join(directory, 'dist/SHA256SUMS'), `${'0'.repeat(64)}  package.tgz\n`);
  const result = spawnSync(process.execPath, [
    fileURLToPath(new URL('../scripts/package-binary.js', import.meta.url))], {
    cwd: directory, encoding: 'utf8',
    env: { ...process.env, NODE_DISTRIBUTION_LICENSE: fileURLToPath(new URL('../LICENSE', import.meta.url)) },
  });
  assert.equal(result.status, 0, result.stderr);
  const sums = readFileSync(path.join(directory, 'dist/SHA256SUMS'), 'utf8');
  assert.match(sums, /package\.tgz/);
  assert.match(sums, process.platform === 'win32' ? /framerize-nexus\.exe\n/ : /framerize-nexus\n/);
  const platformName = { darwin: 'macos', win32: 'windows' }[process.platform] || process.platform;
  const suffix = `${platformName}-${process.arch}`;
  assert.match(sums, new RegExp(`framerize-nexus-${suffix}\\.`));
  const repeated = spawnSync(process.execPath, [
    fileURLToPath(new URL('../scripts/package-binary.js', import.meta.url))], {
    cwd: directory, encoding: 'utf8',
  });
  assert.equal(repeated.status, 0, repeated.stderr);
  const refreshed = readFileSync(path.join(directory, 'dist/SHA256SUMS'), 'utf8');
  assert.match(refreshed, /package\.tgz/);
  assert.equal(refreshed.trim().split('\n').length, 3);
  assert.equal(new Set(refreshed.trim().split('\n').map((line) => line.slice(66))).size, 3);
});

test('binary packaging creates checksums in a clean checkout', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'framer-next-package-clean-'));
  mkdirSync(path.join(directory, 'dist'));
  const executable = process.platform === 'win32' ? 'framerize-nexus.exe' : 'framerize-nexus';
  writeFileSync(path.join(directory, 'dist', executable), 'fixture');
  const result = spawnSync(process.execPath, [
    fileURLToPath(new URL('../scripts/package-binary.js', import.meta.url))], {
    cwd: directory, encoding: 'utf8',
    env: { ...process.env, NODE_DISTRIBUTION_LICENSE: fileURLToPath(new URL('../LICENSE', import.meta.url)) },
  });
  assert.equal(result.status, 0, result.stderr);
  const sums = readFileSync(path.join(directory, 'dist/SHA256SUMS'), 'utf8');
  assert.match(sums, process.platform === 'win32'
    ? /^[a-f0-9]{64}  framerize-nexus\.exe\n/m : /^[a-f0-9]{64}  framerize-nexus\n/m);
});

test('binary packaging normalizes copied license text before archiving', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'framer-next-license-lines-'));
  const nodeLicense = path.join(directory, 'node-license.txt');
  writeFileSync(nodeLicense, 'Node notice\r\nSecond line\r\n');
  packageFixture(directory, nodeLicense);
  const projectLicense = readFileSync(new URL('../LICENSE', import.meta.url), 'utf8')
    .replace(/\r\n/g, '\n');
  assert.equal(readFileSync(path.join(directory, 'dist/LICENSE'), 'utf8'), projectLicense);
  assert.equal(readFileSync(path.join(directory, 'dist/NODE-LICENSE'), 'utf8'),
    'Node notice\nSecond line\n');
  assert.equal(readFileSync(nodeLicense, 'utf8'), 'Node notice\r\nSecond line\r\n');
  const archive = spawnSync('tar', ['-xOf', `dist/${archiveName}`, 'LICENSE'], {
    cwd: directory, encoding: 'utf8',
  });
  assert.equal(archive.status, 0, archive.stderr);
  assert.equal(archive.stdout, projectLicense);
  const nodeArchive = spawnSync('tar', ['-xOf', `dist/${archiveName}`, 'NODE-LICENSE'], {
    cwd: directory, encoding: 'utf8',
  });
  assert.equal(nodeArchive.status, 0, nodeArchive.stderr);
  assert.equal(nodeArchive.stdout, 'Node notice\nSecond line\n');
});
