import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';

const workspace = path.resolve(process.env.RELEASE_WORKSPACE || '.');
const pkg = JSON.parse(readFileSync(path.join(workspace, 'package.json'), 'utf8'));
if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(pkg.version)) {
  throw new Error('package version must be valid SemVer');
}
const tag = process.env.GITHUB_REF_NAME || '';
if (tag && tag !== `framer-next-v${pkg.version}`) {
  throw new Error(`release tag ${tag} does not match package version ${pkg.version}`);
}
const sums = readFileSync(path.join(workspace, 'dist/SHA256SUMS'), 'utf8');
if (!sums.trim()) throw new Error('SHA256SUMS must not be empty');
const artifacts = {};
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
}, null, 2)}\n`);
