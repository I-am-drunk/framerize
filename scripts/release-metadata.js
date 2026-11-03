import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync } from 'node:fs';

const pkg = JSON.parse(readFileSync('package.json', 'utf8'));
const sums = readFileSync('dist/SHA256SUMS', 'utf8');
writeFileSync('dist/release.json', `${JSON.stringify({
  version: pkg.version,
  commit: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
  node: process.version,
  platform: process.platform,
  architecture: process.arch,
  artifacts: sums.trim().split('\n'),
}, null, 2)}\n`);
