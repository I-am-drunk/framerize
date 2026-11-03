import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const directory = mkdtempSync(path.join(tmpdir(), 'framer-next-package-'));
const tarball = execFileSync('npm', ['pack', '--pack-destination', directory, '--silent'], {
  encoding: 'utf8',
}).trim();
execFileSync('npm', ['install', '--ignore-scripts', '--no-audit', '--no-fund',
  path.join(directory, tarball)], { cwd: directory, stdio: 'ignore' });
const bin = path.join(directory, 'node_modules', '.bin', 'framer-next');
const help = execFileSync(bin, ['--help'], { cwd: directory, encoding: 'utf8' });
assert.match(help, /Framer Next/);
await import(path.join(directory, 'node_modules', 'framer-next', 'src', 'config.js'));
console.log('installed npm package smoke passed');
