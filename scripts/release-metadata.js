import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';

const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
const tag = process.env.GITHUB_REF_NAME || '';
if (tag && tag !== `framer-next-v${pkg.version}`) {
  throw new Error(`release tag ${tag} does not match package version ${pkg.version}`);
}
const sums = readFileSync('dist/SHA256SUMS', 'utf8');
const artifacts = Object.fromEntries(sums.trim().split('\n').map((line) => {
  const match = line.match(/^([a-f0-9]{64}) [ *](.+)$/);
  if (!match) throw new Error('invalid SHA256SUMS entry');
  return [match[2], match[1]];
}));
const commit = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
if (process.env.GITHUB_SHA && process.env.GITHUB_SHA !== commit) {
  throw new Error('release source commit does not match the triggering commit');
}
writeFileSync('dist/release.json', `${JSON.stringify({
  version: pkg.version,
  commit,
  node: process.version,
  platform: process.platform,
  architecture: process.arch,
  artifacts,
}, null, 2)}\n`);
