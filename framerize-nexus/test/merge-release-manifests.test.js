import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const commit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
const digest = (value) => createHash('sha256').update(value).digest('hex');
const script = fileURLToPath(new URL('../scripts/merge-release-manifests.js', import.meta.url));

function fixture() {
  const directory = mkdtempSync(path.join(tmpdir(), 'framerize-nexus-manifests-'));
  writeFileSync(path.join(directory, 'package.json'), JSON.stringify({ version: '0.1.9' }));
  mkdirSync(path.join(directory, 'dist'));
  writeFileSync(path.join(directory, 'dist/framerize-nexus-0.1.9.tgz'), 'package');
  for (const platform of ['linux-x64', 'macos-arm64', 'windows-x64']) {
    const source = path.join(directory, 'dist', platform);
    mkdirSync(source);
    const executable = platform === 'windows-x64' ? 'framerize-nexus.exe' : 'framerize-nexus';
    const archive = `framerize-nexus-${platform}.${platform === 'windows-x64' ? 'zip' : 'tar.gz'}`;
    for (const [name, contents] of [[executable, `executable-${platform}`],
      [archive, `archive-${platform}`], ['LICENSE', 'project'], ['NODE-LICENSE', 'node']]) {
      writeFileSync(path.join(source, name), contents);
    }
    writeFileSync(path.join(source, 'release.json'), JSON.stringify({ version: '0.1.9', commit,
      node: 'v22.12.0', artifacts: Object.fromEntries([executable, archive].map((name) =>
        [name, digest(readFileSync(path.join(source, name)))])),
      packageArtifact: { name: 'framerize-nexus-0.1.9.tgz', sha256: digest('package') },
      licenses: Object.fromEntries(['LICENSE', 'NODE-LICENSE'].map((name) =>
        [name, digest(readFileSync(path.join(source, name)))])),
    }));
  }
  return directory;
}

function merge(directory) {
  return spawnSync(process.execPath, [script], { cwd: directory, encoding: 'utf8',
    env: { ...process.env, GITHUB_SHA: commit } });
}

test('release merge hashes exactly the downloadable artifacts', () => {
  const directory = fixture();
  const result = merge(directory);
  assert.equal(result.status, 0, result.stderr);
  const metadata = JSON.parse(readFileSync(path.join(directory, 'dist/release.json'), 'utf8'));
  assert.equal(metadata.commit, commit);
  assert.equal(metadata.builds.length, 3);
  assert.deepEqual(Object.keys(metadata.artifacts).sort(), [
    'LICENSE', 'NODE-LICENSE', 'framerize-nexus-0.1.9.tgz',
    'framerize-nexus-linux-x64.tar.gz', 'framerize-nexus-macos-arm64.tar.gz',
    'framerize-nexus-windows-x64.zip',
  ]);
  for (const line of readFileSync(path.join(directory, 'dist/SHA256SUMS'), 'utf8').trim().split('\n')) {
    const [checksum, name] = line.split('  ');
    assert.equal(digest(readFileSync(path.join(directory, 'dist', name))), checksum);
  }
});

test('release merge rejects a corrupt platform archive', () => {
  const directory = fixture();
  writeFileSync(path.join(directory, 'dist/linux-x64/framerize-nexus-linux-x64.tar.gz'), 'corrupt');
  assert.match(merge(directory).stderr, /release digest mismatch/);
});

test('release merge rejects a missing platform', () => {
  const directory = fixture();
  rmSync(path.join(directory, 'dist/macos-arm64'), { recursive: true });
  assert.notEqual(merge(directory).status, 0);
});

test('release merge rejects a different source commit', () => {
  const directory = fixture();
  const file = path.join(directory, 'dist/windows-x64/release.json');
  const metadata = JSON.parse(readFileSync(file, 'utf8'));
  metadata.commit = 'b'.repeat(40);
  writeFileSync(file, JSON.stringify(metadata));
  assert.match(merge(directory).stderr, /reviewed checkout/);
});

test('release merge rejects unexpected build artifacts', () => {
  const directory = fixture();
  const file = path.join(directory, 'dist/linux-x64/release.json');
  const metadata = JSON.parse(readFileSync(file, 'utf8'));
  metadata.artifacts.extra = '0'.repeat(64);
  writeFileSync(file, JSON.stringify(metadata));
  assert.match(merge(directory).stderr, /unexpected build artifacts/);
});
