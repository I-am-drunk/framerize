import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const names = ['framerize-nexus', 'framerize-server'];
const hash = (file) => createHash('sha256').update(readFileSync(file)).digest('hex');
const npmCandidates = [
  path.join(path.dirname(process.execPath), 'node_modules/npm/bin/npm-cli.js'),
  path.resolve(path.dirname(process.execPath), '../lib/node_modules/npm/bin/npm-cli.js'),
];
const networkGuard = `
import net from 'node:net';
import dgram from 'node:dgram';
import http from 'node:http';
import https from 'node:https';
import { syncBuiltinESMExports } from 'node:module';
const deny = () => { throw new Error('Network disabled during package coexistence check'); };
net.Socket.prototype.connect = deny;
net.Server.prototype.listen = deny;
dgram.Socket.prototype.bind = deny;
dgram.Socket.prototype.send = deny;
http.request = http.get = https.request = https.get = deny;
globalThis.fetch = deny;
syncBuiltinESMExports();
`;

function isolatedEnvironment(directory, guard) {
  const systemRoot = process.env.SystemRoot || process.env.SYSTEMROOT || 'C:\\Windows';
  const systemPaths = process.platform === 'win32'
    ? [systemRoot, path.join(systemRoot, 'System32')] : ['/usr/bin', '/bin'];
  return {
    PATH: [path.dirname(process.execPath), ...systemPaths].join(path.delimiter),
    ...(process.platform === 'win32' ? {
      SystemRoot: systemRoot, ComSpec: path.join(systemRoot, 'System32/cmd.exe'),
      PATHEXT: '.COM;.EXE;.BAT;.CMD',
    } : {}),
    HOME: directory, USERPROFILE: directory, APPDATA: directory,
    LOCALAPPDATA: directory, TMPDIR: directory, TMP: directory, TEMP: directory,
    NODE_OPTIONS: `--import=${JSON.stringify(pathToFileURL(guard).href)}`,
    npm_config_cache: path.join(directory, 'cache'), npm_config_offline: 'true',
    npm_config_userconfig: path.join(directory, 'user.npmrc'),
    npm_config_globalconfig: path.join(directory, 'global.npmrc'),
    npm_config_ignore_scripts: 'true', npm_config_audit: 'false', npm_config_fund: 'false',
  };
}

export function installedBytes(bytes, isLauncher) {
  const newline = bytes.indexOf(10);
  if (isLauncher && bytes.subarray(0, 2).toString() === '#!'
      && newline > 0 && bytes[newline - 1] === 13) {
    return Buffer.concat([bytes.subarray(0, newline - 1), bytes.subarray(newline)]);
  }
  return bytes;
}

export function checkPackageCoexistence(tarballs) {
  assert.equal(tarballs.length, 2, 'provide exactly two existing package tarballs');
  const inputs = tarballs.map((file) => realpathSync(file));
  assert.equal(new Set(inputs).size, 2, 'package tarballs must be distinct');
  const directory = mkdtempSync(path.join(tmpdir(), 'framerize-pair-'));
  try {
    const guard = path.join(directory, 'network-guard.mjs');
    writeFileSync(guard, networkGuard);
    for (const config of ['user.npmrc', 'global.npmrc']) writeFileSync(path.join(directory, config), '');
    const options = { cwd: directory, env: isolatedEnvironment(directory, guard),
      encoding: 'utf8', timeout: 30_000, stdio: ['ignore', 'pipe', 'pipe'] };
    const npmCli = npmCandidates.find((candidate) => existsSync(candidate));
    assert.ok(npmCli, 'npm must be installed beside the selected Node runtime');
    const runNpm = (args) => execFileSync(process.execPath, [npmCli, ...args], options);
    const runNode = (args) => execFileSync(process.execPath, args, options);
    const archiveBytes = (file, member) => execFileSync('tar', ['-xOf', file, member],
      { ...options, encoding: null });
    const packages = inputs.map((file, index) => {
      const bytes = readFileSync(file);
      const snapshot = path.join(directory, `input-${index}.tgz`);
      writeFileSync(snapshot, bytes);
      const manifest = JSON.parse(archiveBytes(snapshot, 'package/package.json'));
      return { file, snapshot, manifest, sha256: hash(snapshot) };
    });
    assert.deepEqual(packages.map(({ manifest }) => manifest.name).sort(), names,
      'expected one Nexus and one server package');
    for (const entry of packages) {
      const { manifest, snapshot } = entry;
      for (const field of ['dependencies', 'optionalDependencies', 'peerDependencies']) {
        assert.equal(Object.keys(manifest[field] || {}).length, 0, `${manifest.name} has ${field}`);
      }
      assert.deepEqual(Object.keys(manifest.bin || {}), [manifest.name], 'unexpected executable name');
      const launcher = manifest.bin[manifest.name].replace(/^\.\//, '');
      assert.equal(launcher, `bin/${manifest.name}.js`, 'unexpected executable path');
      entry.launcher = launcher;
      entry.files = execFileSync('tar', ['-tzf', snapshot], options).trim().split(/\r?\n/)
        .filter((member) => !member.endsWith('/'));
      assert.equal(new Set(entry.files).size, entry.files.length, 'duplicate archive entries');
      assert.ok(entry.files.includes(`package/${launcher}`), 'launcher missing from tarball');
      for (const member of entry.files) {
        assert.match(member, /^package\/[a-zA-Z0-9_./-]+$/, 'unsupported archive member');
        assert.ok(!member.split('/').some((part) => !part || part === '.' || part === '..'),
          'unsafe archive member');
      }
    }
    writeFileSync(path.join(directory, 'package.json'), '{"private":true}');
    runNpm(['install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund',
      ...packages.map(({ snapshot }) => snapshot)]);
    const results = [];
    const launchers = [];
    for (const { file, snapshot, manifest, sha256, launcher, files } of packages) {
      const installed = path.join(directory, 'node_modules', manifest.name);
      const executable = path.join(installed, launcher);
      launchers.push(realpathSync(executable));
      for (const member of files) {
        const relative = member.slice('package/'.length);
        const expected = archiveBytes(snapshot, member);
        const actual = readFileSync(path.join(installed, relative));
        assert.deepEqual(actual, installedBytes(expected, relative === launcher),
          `${manifest.name}/${relative} differs from the tarball`);
      }
      const link = path.join(directory, 'node_modules/.bin',
        `${manifest.name}${process.platform === 'win32' ? '.cmd' : ''}`);
      assert.ok(existsSync(link), 'installed executable missing');
      if (process.platform !== 'win32') assert.equal(realpathSync(link), realpathSync(executable));
      const title = manifest.name === 'framerize-nexus' ? /Framerize Nexus/ : /Framerize server/;
      assert.match(runNode([executable, '--help']), title);
      assert.match(runNpm(['exec', '--offline', '--no', '--', manifest.name, '--help']), title);
      runNode([executable, 'check', '--host', '127.0.0.1', '--upstream', 'http://127.0.0.1:1']);
      assert.equal(hash(file), sha256, 'input tarball changed during verification');
      results.push({ name: manifest.name, version: manifest.version,
        tarball: path.basename(file), sha256, files: files.length });
    }
    assert.equal(new Set(launchers).size, 2, 'executables must remain distinct');
    runNode(['--input-type=module', '-e', `
      import assert from 'node:assert/strict';
      import { createServer as server } from 'framerize-server';
      import { createServer as nexus } from 'framerize-nexus';
      import { loadConfig } from 'framerize-server/config';
      import { parseConfig } from 'framerize-nexus/config';
      import 'framerize-server/accounts';
      import 'framerize-server/logger';
      import 'framerize-server/provider';
      assert.equal(typeof server, 'function');
      assert.equal(typeof nexus, 'function');
      assert.notEqual(server, nexus);
      assert.equal(loadConfig().port, 8789);
      assert.equal(parseConfig().port, 8788);
    `]);
    return { node: process.version, npm: runNpm(['--version']).trim(),
      platform: process.platform, arch: process.arch, packages: results };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    console.log(JSON.stringify(checkPackageCoexistence(process.argv.slice(2)), null, 2));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
