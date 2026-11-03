import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

test('binary packaging preserves existing checksums and adds the archive', () => {
  const directory = mkdtempSync(path.join(tmpdir(), 'framer-next-package-'));
  mkdirSync(path.join(directory, 'dist'));
  writeFileSync(path.join(directory, 'dist/framer-next'), 'fixture');
  writeFileSync(path.join(directory, 'dist/SHA256SUMS'), `${'0'.repeat(64)}  package.tgz\n`);
  const result = spawnSync(process.execPath, [
    new URL('../scripts/package-binary.js', import.meta.url).pathname], {
    cwd: directory, encoding: 'utf8',
  });
  assert.equal(result.status, 0, result.stderr);
  const sums = readFileSync(path.join(directory, 'dist/SHA256SUMS'), 'utf8');
  assert.match(sums, /package\.tgz/);
  assert.match(sums, /framer-next\n/);
  assert.match(sums, /framer-next-linux-x64\.tar\.gz/);
});
