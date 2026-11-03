import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

export const targets = Object.freeze({
  'linux-x64': { platform: 'linux', architecture: 'x64' },
  'macos-arm64': { platform: 'darwin', architecture: 'arm64' },
  'windows-x64': { platform: 'win32', architecture: 'x64' },
});

export function artifactNames(target) {
  if (!Object.hasOwn(targets, target)) throw new Error('Unsupported executable target');
  const windows = target === 'windows-x64';
  return [`framerize-server${windows ? '.exe' : ''}`,
    `framerize-server-${target}.${windows ? 'zip' : 'tar.gz'}`];
}

export function currentTarget() {
  const target = Object.keys(targets).find((name) => targets[name].platform === process.platform &&
    targets[name].architecture === process.arch);
  if (!target) throw new Error('Unsupported executable target');
  return target;
}

export const digest = (file) => createHash('sha256').update(readFileSync(file)).digest('hex');

export function verifyLicenses(directory) {
  for (const name of ['LICENSE', 'NODE-LICENSE']) {
    const content = readFileSync(path.join(directory, name), 'utf8');
    if (!content.trim() || content.includes('\r')) throw new Error('License must be nonempty canonical LF text');
    if (name === 'LICENSE' && content !== readFileSync('LICENSE', 'utf8').replace(/\r\n/g, '\n')) {
      throw new Error('Project license differs from checkout');
    }
  }
}

export function writeSums(directory, artifacts) {
  writeFileSync(path.join(directory, 'SHA256SUMS'), Object.entries(artifacts)
    .map(([name, checksum]) => `${checksum}  ${name}\n`).join(''));
}

export function sourceIdentity() {
  const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
  const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;
  if (pkg.name !== 'framerize-server' || pkg.private !== true || !semver.test(pkg.version)) {
    throw new Error('Invalid private server package identity');
  }
  const git = (args) => execFileSync('git', args, { encoding: 'utf8' }).trim();
  const commit = git(['rev-parse', 'HEAD']);
  if (!/^[a-f0-9]{40}$/.test(commit) || (process.env.GITHUB_SHA && process.env.GITHUB_SHA !== commit)) {
    throw new Error('Source commit does not match the checkout');
  }
  const inputs = ['.', '../.github/workflows/server-ci.yml', '../framerize-core',
    '../scripts/build-server.ts', '../scripts/server-native-bundle.ts',
    '../scripts/package-archive.ts', '../package.json', '../package-lock.json',
    '../tsconfig.base.json', '../tsconfig.tools.json', '../tsconfig.server-tools.json'];
  if (git(['status', '--porcelain', '--untracked-files=normal', '--', ...inputs])) {
    throw new Error('Packaging requires a clean committed server checkout');
  }
  return { name: pkg.name, version: pkg.version, commit };
}

export function verifySums(directory, artifacts) {
  const expected = Object.entries(artifacts).map(([name, checksum]) => `${checksum}  ${name}`).sort();
  const actual = readFileSync(path.join(directory, 'SHA256SUMS'), 'utf8').trim().split('\n').sort();
  if (JSON.stringify(actual) !== JSON.stringify(expected)) throw new Error('Checksum manifest mismatch');
  for (const [name, checksum] of Object.entries(artifacts)) {
    if (path.basename(name) !== name || !/^[a-f0-9]{64}$/.test(checksum) ||
        digest(path.join(directory, name)) !== checksum) throw new Error('Artifact digest mismatch');
  }
}
