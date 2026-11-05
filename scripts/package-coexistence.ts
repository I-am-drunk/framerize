import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import type { ExecFileSyncOptionsWithStringEncoding } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, lstatSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { readPackageArchive } from './package-archive.js';

const names = ['framerize-remote', 'framerize-server'];
function record(value: unknown): asserts value is Record<string, unknown> {
  assert.ok(value && typeof value === 'object' && !Array.isArray(value), 'Invalid package manifest');
}

function manifestOf(bytes: Buffer) {
  const value: unknown = JSON.parse(bytes.toString('utf8'));
  record(value);
  assert.equal(typeof value.name, 'string');
  assert.equal(typeof value.version, 'string');
  const name = value.name as string;
  const version = value.version as string;
  assert.match(version, /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/, 'Invalid package version');
  record(value.bin);
  for (const field of ['dependencies', 'optionalDependencies', 'peerDependencies']) {
    if (value[field] !== undefined) record(value[field]);
    assert.equal(Object.keys(value[field] ?? {}).length, 0, `${name} has ${field}`);
  }
  return { name, version, bin: value.bin };
}
const hash = (file: string) => createHash('sha256').update(readFileSync(file)).digest('hex');
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

function isolatedEnvironment(directory: string, guard: string): NodeJS.ProcessEnv {
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

export function installedBytes(bytes: Buffer, isLauncher: boolean): Buffer {
  const newline = bytes.indexOf(10);
  if (isLauncher && bytes.subarray(0, 2).toString() === '#!'
      && newline > 0 && bytes[newline - 1] === 13) {
    return Buffer.concat([bytes.subarray(0, newline - 1), bytes.subarray(newline)]);
  }
  return bytes;
}

export function checkPackageCoexistence(tarballs: readonly string[]) {
  assert.equal(tarballs.length, 2, 'provide exactly two existing package tarballs');
  const inputs = tarballs.map((file) => realpathSync(file));
  assert.equal(new Set(inputs).size, 2, 'package tarballs must be distinct');
  const directory = mkdtempSync(path.join(tmpdir(), 'framerize-pair-'));
  try {
    const guard = path.join(directory, 'network-guard.mjs');
    writeFileSync(guard, networkGuard);
    for (const config of ['user.npmrc', 'global.npmrc']) writeFileSync(path.join(directory, config), '');
    const options: ExecFileSyncOptionsWithStringEncoding = { cwd: directory, env: isolatedEnvironment(directory, guard),
      encoding: 'utf8', timeout: 30_000, stdio: ['ignore', 'pipe', 'pipe'] };
    const npmCli = npmCandidates.find((candidate) => existsSync(candidate));
    assert.ok(npmCli, 'npm must be installed beside the selected Node runtime');
    const runNpm = (args: string[]) => execFileSync(process.execPath, [npmCli, ...args], options);
    const runNode = (args: string[]) => execFileSync(process.execPath, args, options);
    const packages = inputs.map((file, index) => {
      assert.ok(lstatSync(file).isFile() && lstatSync(file).size <= 16 * 1024 * 1024, 'Invalid archive size or type');
      const bytes = readFileSync(file);
      const snapshot = path.join(directory, `input-${index}.tgz`);
      writeFileSync(snapshot, bytes);
      const archive = readPackageArchive(snapshot);
      const manifest = manifestOf(archive.get('package/package.json')!);
      return { file, snapshot, manifest, archive, sha256: hash(snapshot), launcher: '', files: [] as string[] };
    });
    assert.deepEqual(packages.map(({ manifest }) => manifest.name).sort(), names,
      'expected one Remote and one server package');
    for (const entry of packages) {
      const { manifest, archive } = entry;
      assert.deepEqual(Object.keys(manifest.bin || {}), [manifest.name], 'unexpected executable name');
      const executable = manifest.bin[manifest.name];
      assert.equal(typeof executable, 'string', 'Invalid executable path');
      const launcher = (executable as string).replace(/^\.\//, '');
      assert.equal(launcher, `bin/${manifest.name}.js`, 'unexpected executable path');
      entry.launcher = launcher;
      entry.files = [...archive.keys()];
      assert.ok(entry.files.includes(`package/${launcher}`), 'launcher missing from tarball');
    }
    writeFileSync(path.join(directory, 'package.json'), '{"private":true}');
    runNpm(['install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund',
      ...packages.map(({ snapshot }) => snapshot)]);
    const results = [];
    const launchers = [];
    for (const { file, archive, manifest, sha256, launcher, files } of packages) {
      const installed = path.join(directory, 'node_modules', manifest.name);
      const executable = path.join(installed, launcher);
      launchers.push(realpathSync(executable));
      const installedFiles = readdirSync(installed, { recursive: true, encoding: 'utf8' }).filter(relative => {
        const info = lstatSync(path.join(installed, relative));
        assert.ok(info.isFile() || info.isDirectory(), 'Installed package contains a link or special file');
        return info.isFile();
      }).map(relative => `package/${relative.replaceAll('\\', '/')}`).sort();
      assert.deepEqual(installedFiles, [...files].sort(), 'Installed inventory differs from archive');
      for (const member of files) {
        const relative = member.slice('package/'.length);
        const expected = archive.get(member)!;
        const actual = readFileSync(path.join(installed, relative));
        assert.deepEqual(actual, installedBytes(expected, relative === launcher),
          `${manifest.name}/${relative} differs from the tarball`);
      }
      const link = path.join(directory, 'node_modules/.bin',
        `${manifest.name}${process.platform === 'win32' ? '.cmd' : ''}`);
      assert.ok(existsSync(link), 'installed executable missing');
      if (process.platform !== 'win32') assert.equal(realpathSync(link), realpathSync(executable));
      const title = manifest.name === 'framerize-remote' ? /framerize-remote/ : /^Server\b/m;
      assert.match(runNode([executable, '--help']), title);
      assert.match(runNpm(['exec', '--offline', '--no', '--', manifest.name, '--help']), title);
      assert.match(runNode([path.join(path.dirname(npmCli), 'npx-cli.js'), '--offline', '--no-install',
        manifest.name, '--help']), title);
      if (manifest.name === 'framerize-remote') {
        assert.equal(runNode([executable, 'version']).trim(), manifest.version);
        assert.match(runNode([executable, 'setup', '--help']), /--server-token-env/);
      } else runNode([executable, 'check', '--host', '127.0.0.1', '--upstream', 'http://127.0.0.1:1']);
      assert.equal(hash(file), sha256, 'input tarball changed during verification');
      results.push({ name: manifest.name, version: manifest.version,
        tarball: path.basename(file), sha256, files: files.length,
        inventory: [...archive].map(([member, bytes]) => ({ path: member,
          bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') })) });
    }
    assert.equal(new Set(launchers).size, 2, 'executables must remain distinct');
    runNode(['--input-type=module', '-e', `
      import assert from 'node:assert/strict';
      import { createServer as server } from 'framerize-server';
      import { createRemoteBackend, parseRemoteEndpoint } from 'framerize-remote';
      import { loadConfig } from 'framerize-server/config';
      import 'framerize-server/accounts';
      import 'framerize-server/logger';
      import 'framerize-server/provider';
      assert.equal(typeof server, 'function');
      assert.equal(typeof createRemoteBackend, 'function');
      assert.notEqual(server, createRemoteBackend);
      assert.equal(loadConfig().port, 8789);
      const backend = createRemoteBackend({ endpoint: parseRemoteEndpoint('http://127.0.0.1', false),
        serverToken: 'synthetic', headersTimeoutMs: 100, idleTimeoutMs: 100,
        readinessWholeMs: 200, readinessBodyMaxBytes: 65536 });
      assert.equal(typeof backend.ready, 'function');
      assert.equal(typeof backend.forward, 'function');
      await backend.close();
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
    console.error(error instanceof Error ? error.message : 'Package coexistence verification failed');
    process.exitCode = 1;
  }
}
