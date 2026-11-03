import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';

const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
const sums = readFileSync('dist/SHA256SUMS', 'utf8');
const artifacts = Object.fromEntries(sums.trim().split('\n').map((line) => {
  const match = line.match(/^([a-f0-9]{64}) [ *](.+)$/);
  if (!match) throw new Error('invalid SHA256SUMS entry');
  return [match[2], match[1]];
}));
writeFileSync('dist/release.json', `${JSON.stringify({
  version: pkg.version,
  commit: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
  node: process.version,
  platform: process.platform,
  architecture: process.arch,
  artifacts,
}, null, 2)}\n`);
