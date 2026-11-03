import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { copyFileSync, existsSync, lstatSync, mkdirSync, mkdtempSync, readFileSync,
  readdirSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

interface PackInfo {
  filename: string;
  size: number;
  unpackedSize: number;
  files: { path: string; size: number }[];
}

export function packagePolicy(info: PackInfo, expected: string[]): void {
  assert.equal(basename(info.filename), info.filename, 'Unsafe archive path');
  assert.match(info.filename, /^framerize-core-[\w.-]+\.tgz$/);
  assert.ok(Number.isSafeInteger(info.size) && info.size > 0 && info.size <= 64 * 1024,
    'Packed core exceeds 64 KiB');
  assert.ok(Number.isSafeInteger(info.unpackedSize) && info.unpackedSize > 0 && info.unpackedSize <= 256 * 1024,
    'Unpacked core exceeds 256 KiB');
  assert.deepEqual(info.files.map((file) => file.path).sort(), [...expected].sort(),
    'Unexpected package inventory');
}

export function npmCli(): string {
  const binary = dirname(process.execPath);
  const candidates = [join(binary, 'node_modules/npm/bin/npm-cli.js'),
    join(binary, '../lib/node_modules/npm/bin/npm-cli.js')];
  const found = candidates.find(existsSync);
  assert.ok(found, 'Cannot locate the current Node installation npm CLI');
  return realpathSync(found);
}

export function privateEnvironment(root: string): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = {};
  for (const name of ['PATH', 'Path', 'SystemRoot', 'WINDIR', 'ComSpec', 'PATHEXT']) {
    if (process.env[name] !== undefined) env[name] = process.env[name];
  }
  return { ...env, HOME: root, USERPROFILE: root, TMPDIR: root, TMP: root, TEMP: root,
    NODE_OPTIONS: '', NODE_PATH: '', NODE_DISABLE_COMPILE_CACHE: '1',
    npm_config_userconfig: join(root, 'user.npmrc'),
    npm_config_globalconfig: join(root, 'global.npmrc'),
    npm_config_cache: join(root, 'cache'), npm_config_offline: 'true',
    npm_config_ignore_scripts: 'true', npm_config_audit: 'false',
    npm_config_fund: 'false', npm_config_update_notifier: 'false' };
}

export function checkCorePackage(repository: string): object {
  const root = mkdtempSync(join(tmpdir(), 'framerize-core-package-'));
  try {
    writeFileSync(join(root, 'user.npmrc'), '');
    writeFileSync(join(root, 'global.npmrc'), '');
    const env = privateEnvironment(root);
    const run = (file: string, args: string[], cwd: string) => execFileSync(file, args,
      { cwd, env, encoding: 'utf8', timeout: 60_000, maxBuffer: 2 * 1024 * 1024 });
    return verifyPackage(repository, root, run);
  } finally { rmSync(root, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 }); }
}

type Runner = (file: string, args: string[], cwd: string) => string;

function verifyPackage(repository: string, root: string, run: Runner): object {
  const source = join(repository, 'framerize-core');
  const manifest = JSON.parse(readFileSync(join(source, 'package.json'), 'utf8'));
  assert.equal(manifest.name, '@framerize/core');
  assert.equal(manifest.private, true);
  for (const key of ['dependencies', 'optionalDependencies', 'peerDependencies']) {
    assert.equal(Object.keys(manifest[key] ?? {}).length, 0, 'Core must have no runtime dependencies');
  }
  const expected = ['README.md', 'package.json', ...readdirSync(join(source, 'dist'))
    .map((name) => `dist/${name}`)];
  for (const name of expected) {
    assert.ok(lstatSync(join(source, name)).isFile(), 'Package inputs must be regular files');
    assert.ok(name === 'README.md' || name === 'package.json' || /^dist\/[\w-]+\.(js|d\.ts)$/.test(name));
  }
  const npm = npmCli();
  const packed = JSON.parse(run(process.execPath, [npm, 'pack', '--offline', '--ignore-scripts',
    '--json', '--pack-destination', root], source)) as PackInfo[];
  assert.equal(packed.length, 1);
  const info = packed[0]!;
  packagePolicy(info, expected);
  const archive = join(root, info.filename);
  assert.equal(readFileSync(archive).length, info.size, 'Archive size differs from npm metadata');
  assert.equal(info.unpackedSize, info.files.reduce((total, file) => total + file.size, 0));
  for (const file of info.files) assert.equal(readFileSync(join(source, file.path)).length,
    file.size, 'Package member size differs from source');
  const rebuild = verifyRebuild(source, root, info, run);
  const consumer = join(root, 'consumer');
  mkdirSync(consumer);
  writeFileSync(join(consumer, 'package.json'), JSON.stringify({
    name: 'isolated-core-check', private: true, type: 'module',
  }));
  run(process.execPath, [npm, 'install', '--offline', '--ignore-scripts', '--no-audit',
    '--no-fund', '--package-lock=false', archive], consumer);
  return { ...verifyInstalled(source, consumer, root, info, run), rebuild };
}

function verifyRebuild(source: string, root: string, original: PackInfo, run: Runner) {
  const destination = join(root, 'rebuild');
  const staged = join(destination, 'framerize-core');
  mkdirSync(join(staged, 'src'), { recursive: true });
  copyFileSync(join(dirname(source), 'tsconfig.base.json'), join(destination, 'tsconfig.base.json'));
  for (const name of ['package.json', 'README.md', 'tsconfig.build.json']) {
    copyFileSync(join(source, name), join(staged, name));
  }
  for (const name of readdirSync(join(source, 'src'))) {
    assert.match(name, /^[\w-]+\.ts$/, 'Unexpected core source file');
    assert.ok(lstatSync(join(source, 'src', name)).isFile(), 'Core source must be a regular file');
    copyFileSync(join(source, 'src', name), join(staged, 'src', name));
  }
  const tooling = fileURLToPath(new URL('../../node_modules/', import.meta.url));
  run(process.execPath, [join(tooling, 'typescript/bin/tsc'), '-p',
    join(staged, 'tsconfig.build.json'), '--typeRoots', join(tooling, '@types')], destination);
  const rebuilt = JSON.parse(run(process.execPath, [npmCli(), 'pack', '--offline',
    '--ignore-scripts', '--json', '--pack-destination', destination], staged)) as PackInfo[];
  assert.equal(rebuilt.length, 1);
  packagePolicy(rebuilt[0]!, original.files.map((file) => file.path));
  const digest = (path: string) => createHash('sha256').update(readFileSync(path)).digest('hex');
  const sha256 = digest(join(destination, rebuilt[0]!.filename));
  assert.equal(sha256, digest(join(root, original.filename)), 'Core archive differs from a clean build');
  const compiler = JSON.parse(readFileSync(join(tooling, 'typescript/package.json'), 'utf8'));
  return { sha256, typescript: compiler.version as string };
}

function verifyInstalled(source: string, consumer: string, root: string,
  info: PackInfo, run: Runner): object {
  const installed = join(consumer, 'node_modules/@framerize/core');
  assert.equal(lstatSync(installed).isSymbolicLink(), false, 'Installed core must not be a workspace link');
  const files = readdirSync(installed, { recursive: true, encoding: 'utf8' }).filter((name) => {
    const stat = lstatSync(join(installed, name));
    assert.equal(stat.isSymbolicLink(), false, 'No package symlinks');
    return stat.isFile();
  }).map((name) => name.replaceAll('\\', '/')).sort();
  assert.deepEqual(files, info.files.map((file) => file.path).sort());
  for (const name of files) assert.deepEqual(readFileSync(join(installed, name)),
    readFileSync(join(source, name)), `Installed bytes differ: ${name}`);
  const fixture = fileURLToPath(new URL('./fixtures/core-installed.js', import.meta.url));
  copyFileSync(fixture, join(consumer, 'installed.mjs'));
  const smoke: unknown = JSON.parse(run(process.execPath, ['installed.mjs'], consumer));
  const commit = run('git', ['rev-parse', 'HEAD'], dirname(source)).trim();
  const dirty = run('git', ['status', '--porcelain'], dirname(source)).trim() !== '';
  if (process.env.GITHUB_SHA) {
    assert.equal(commit, process.env.GITHUB_SHA, 'Wrong checkout');
    assert.equal(dirty, false, 'CI checkout changed');
  }
  const npm = run(process.execPath, [npmCli(), '--version'], root).trim();
  return { source: { commit, dirty }, node: process.version, npm, platform: process.platform,
    arch: process.arch, files: files.length, packedBytes: info.size, unpackedBytes: info.unpackedSize,
    sha256: createHash('sha256').update(readFileSync(join(root, info.filename))).digest('hex'), smoke };
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  console.log(JSON.stringify(checkCorePackage(fileURLToPath(new URL('../../', import.meta.url))), null, 2));
}
