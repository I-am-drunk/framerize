import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const directory = mkdtempSync(path.join(tmpdir(), 'framerize-nexus-package-'));
try {
  const downloaded = process.argv.includes('--downloaded');
  const suppliedTarballs = readdirSync('.').filter((name) => /^framerize-nexus-.+\.tgz$/.test(name));
  if (downloaded && suppliedTarballs.length !== 1) {
    throw new Error('exactly one downloaded Nexus tarball is required');
  }
  const suppliedTarball = process.argv[2] && !downloaded
    ? path.resolve(process.argv[2]) : downloaded ? path.resolve(suppliedTarballs[0]) : null;
  const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm';
  const npmCli = process.env.npm_execpath
    || path.join(path.dirname(process.execPath), 'node_modules', 'npm', 'bin', 'npm-cli.js');
  const runNpm = (args, options) => process.platform === 'win32'
    ? execFileSync(process.execPath, [npmCli, ...args], options)
    : execFileSync(npm, args, options);
  const tarball = suppliedTarball ? null : runNpm(['pack', '--pack-destination', directory, '--silent'],
    { encoding: 'utf8' }).trim();
  const tarballPath = suppliedTarball || path.join(directory, tarball);
  runNpm(['install', '--ignore-scripts', '--no-audit', '--no-fund',
    tarballPath], { cwd: directory, stdio: 'ignore' });
  const bin = path.join(directory, 'node_modules', '.bin',
    `framerize-nexus${process.platform === 'win32' ? '.cmd' : ''}`);
  const help = execFileSync(bin, ['--help'], { cwd: directory, encoding: 'utf8', shell: process.platform === 'win32' });
  assert.match(help, /Framerize Nexus/);
  const npxHelp = runNpm(['exec', '--offline', '--no', '--', 'framerize-nexus', '--help'],
    { cwd: directory, encoding: 'utf8' });
  assert.match(npxHelp, /Framerize Nexus/);
  await import(pathToFileURL(path.join(directory, 'node_modules', 'framerize-nexus', 'src', 'config.js')));
  console.log('installed npm package smoke passed');
} finally {
  rmSync(directory, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 });
}
