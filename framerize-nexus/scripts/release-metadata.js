import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { readFileSync, readdirSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const workspace = path.resolve(process.env.RELEASE_WORKSPACE || '.');
const pkg = JSON.parse(readFileSync(path.join(workspace, 'package.json'), 'utf8'));
const semver = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-((?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|\d*[A-Za-z-][0-9A-Za-z-]*))*))?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;
if (typeof pkg.version !== 'string' || !semver.test(pkg.version)) {
  throw new Error('package version must be valid SemVer');
}
const tag = process.env.GITHUB_REF_NAME || '';
if (process.env.GITHUB_REF_TYPE === 'tag' && tag !== `framerize-nexus-v${pkg.version}`) {
  throw new Error(`release tag ${tag} does not match package version ${pkg.version}`);
}
const sums = readFileSync(path.join(workspace, 'dist/SHA256SUMS'), 'utf8');
if (!sums.trim()) throw new Error('SHA256SUMS must not be empty');
const artifacts = {};
const tarballs = readdirSync(workspace).filter((name) => /^framerize-nexus-.+\.tgz$/.test(name));
if (tarballs.length !== 1) throw new Error('exactly one shared Nexus tarball is required');
const packageArtifact = { name: tarballs[0],
  sha256: createHash('sha256').update(readFileSync(path.join(workspace, tarballs[0]))).digest('hex') };
for (const line of sums.trim().split('\n')) {
  const match = line.match(/^([a-f0-9]{64}) [ *](.+)$/);
  if (!match) throw new Error('invalid SHA256SUMS entry');
  if (Object.hasOwn(artifacts, match[2])) throw new Error(`duplicate artifact: ${match[2]}`);
  artifacts[match[2]] = match[1];
}
const commit = process.env.GITHUB_SHA || execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
if (!/^[a-f0-9]{40}$/.test(commit)) throw new Error('release source commit must be a full SHA');
const sourceCommit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
if (process.env.GITHUB_SHA && process.env.GITHUB_SHA !== sourceCommit) {
  throw new Error('release source commit does not match the triggering commit');
}
writeFileSync(path.join(workspace, 'dist/release.json'), `${JSON.stringify({
  version: pkg.version,
  commit,
  node: process.version,
  platform: process.platform,
  architecture: process.arch,
  artifacts,
  packageArtifact,
  licenses: Object.fromEntries(['NODE-LICENSE', 'LICENSE'].map((name) => [name,
    createHash('sha256').update(readFileSync(path.join(workspace, 'dist', name))).digest('hex')])),
}, null, 2)}\n`);
