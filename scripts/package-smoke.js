import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const directory = mkdtempSync(path.join(tmpdir(), 'framer-next-package-'));
const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';
const npmCli = process.env.npm_execpath
  || path.join(path.dirname(process.execPath), 'node_modules', 'npm', 'bin', 'npm-cli.js');
const runNpm = (args, options) => process.platform === 'win32'
  ? execFileSync(process.execPath, [npmCli, ...args], options)
  : execFileSync(npm, args, options);
const tarball = runNpm(['pack', '--pack-destination', directory, '--silent'],
  { encoding: 'utf8' }).trim();
runNpm(['install', '--ignore-scripts', '--no-audit', '--no-fund',
  path.join(directory, tarball)], { cwd: directory, stdio: 'ignore' });
const bin = path.join(directory, 'node_modules', '.bin',
  `framer-next${process.platform === 'win32' ? '.cmd' : ''}`);
const help = execFileSync(bin, ['--help'], { cwd: directory, encoding: 'utf8', shell: process.platform === 'win32' });
assert.match(help, /Framer Next/);
await import(pathToFileURL(path.join(directory, 'node_modules', 'framer-next', 'src', 'config.js')));
console.log('installed npm package smoke passed');
