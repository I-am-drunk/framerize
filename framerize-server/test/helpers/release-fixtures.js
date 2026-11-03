import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, copyFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { artifactNames, currentTarget, digest, writeSums } from '../../scripts/release-artifacts.js';

export const NODE = 'v22.23.4';
export const PACKAGE = 'framerize-server-0.1.0.tgz';
const scriptDirectory = fileURLToPath(new URL('../../scripts/', import.meta.url));
const run = (command, args, options = {}) => execFileSync(command, args, { encoding: 'utf8', ...options });

export function fixture(t) {
  const root = mkdtempSync(path.join(tmpdir(), 'framerize-release-fixture-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const repository = path.join(root, 'checkout');
  const directory = path.join(repository, 'framerize-server');
  mkdirSync(path.join(directory, 'scripts'), { recursive: true });
  mkdirSync(path.join(repository, '.github', 'workflows'), { recursive: true });
  for (const name of ['release-artifacts.js', 'release-metadata.js', 'merge-release-manifests.js', 'package-binary.js']) {
    copyFileSync(path.join(scriptDirectory, name), path.join(directory, 'scripts', name));
  }
  writeFileSync(path.join(directory, 'package.json'), JSON.stringify({ name: 'framerize-server', version: '0.1.0', private: true }));
  writeFileSync(path.join(directory, 'LICENSE'), 'Project license\n');
  writeFileSync(path.join(directory, '.gitignore'), 'dist/\n*.tgz\n');
  writeFileSync(path.join(repository, '.github', 'workflows', 'server-ci.yml'), 'name: fixture\n');
  const git = (args) => run('git', args, { cwd: repository });
  git(['init', '-q']);
  git(['config', 'user.email', 'fixture@example.invalid']);
  git(['config', 'user.name', 'Fixture']);
  git(['add', '.']);
  git(['commit', '-qm', 'fixture']);
  const commit = git(['rev-parse', 'HEAD']).trim();
  return { root, directory, git, commit };
}

export function runScript(directory, script, env = {}) {
  return spawnSync(process.execPath, [path.join(directory, 'scripts', script)], {
    cwd: directory, env: { ...process.env, ...env }, encoding: 'utf8', timeout: 5_000,
  });
}

export function assertFailure(result, message) {
  if (result.status === 0 || result.error || result.signal) throw new Error(`Script did not fail cleanly: ${result.stdout}\n${result.stderr}`);
  if (!result.stderr.includes(message)) throw new Error(`Missing ${message}: ${result.stderr}`);
}

export function seedPackage(directory, bytes = 'shared package\n') {
  writeFileSync(path.join(directory, PACKAGE), bytes);
}

export function seedBuild(t, options = {}) {
  const checkout = fixture(t);
  const { directory, commit } = checkout;
  const target = currentTarget();
  const [executable, archive] = artifactNames(target);
  const dist = path.join(directory, 'dist');
  mkdirSync(dist);
  writeFileSync(path.join(dist, executable), options.executable ?? 'synthetic executable\n');
  writeFileSync(path.join(dist, archive), options.archive ?? 'synthetic archive\n');
  writeFileSync(path.join(dist, 'LICENSE'), options.license ?? 'Project license\n');
  writeFileSync(path.join(dist, 'NODE-LICENSE'), options.nodeLicense ?? 'Node license\n');
  seedPackage(directory, options.packageBytes);
  writeFileSync(path.join(dist, 'build.json'), JSON.stringify({
    name: 'framerize-server', version: '0.1.0', commit,
    target, node: process.version, executableSha256: digest(path.join(dist, executable)),
    packageSha256: digest(path.join(directory, PACKAGE)),
    packageBytes: readFileSync(path.join(directory, PACKAGE)).length,
  }, null, 2));
  writeSums(dist, Object.fromEntries([executable, archive].map((name) => [name, digest(path.join(dist, name))])));
  return { ...checkout, dist, target, executable, archive };
}

export function seedTarget(directory, target, commit, options = {}) {
  const windows = target === 'windows-x64';
  const executable = `framerize-server${windows ? '.exe' : ''}`;
  const archive = `framerize-server-${target}.${windows ? 'zip' : 'tar.gz'}`;
  const dist = path.join(directory, target);
  mkdirSync(dist, { recursive: true });
  for (const [name, bytes] of [
    [executable, `executable ${target}\n`], [archive, `archive ${target}\n`],
    ['LICENSE', options.license ?? 'Project license\n'],
    ['NODE-LICENSE', options.nodeLicense ?? 'Node license\n'],
    [PACKAGE, options.packageBytes ?? 'shared package\n'],
  ]) writeFileSync(path.join(dist, name), bytes);
  const artifacts = Object.fromEntries([executable, archive, PACKAGE, 'LICENSE', 'NODE-LICENSE']
    .map((name) => [name, digest(path.join(dist, name))]));
  writeFileSync(path.join(dist, 'release.json'), JSON.stringify({
    name: 'framerize-server', version: '0.1.0', commit, target,
    node: options.node ?? NODE, artifacts,
    packageSha256: artifacts[PACKAGE], packageBytes: readFileSync(path.join(dist, PACKAGE)).length,
  }, null, 2));
  writeSums(dist, artifacts);
  return { dist, executable, archive, artifacts };
}

export function seedRelease(t) {
  const checkout = fixture(t);
  const { directory, commit } = checkout;
  const dist = path.join(directory, 'dist');
  mkdirSync(dist);
  seedPackage(directory);
  seedPackage(dist);
  const targets = Object.fromEntries(['linux-x64', 'macos-arm64', 'windows-x64']
    .map((target) => [target, seedTarget(dist, target, commit)]));
  return { ...checkout, dist, targets };
}

export function refreshTarget(target) {
  for (const [name, checksum] of Object.entries(target.artifacts)) {
    target.artifacts[name] = digest(path.join(target.dist, name));
  }
  writeSums(target.dist, target.artifacts);
  const metadata = JSON.parse(readFileSync(path.join(target.dist, 'release.json'), 'utf8'));
  metadata.artifacts = target.artifacts;
  writeFileSync(path.join(target.dist, 'release.json'), JSON.stringify(metadata, null, 2));
}

export function runMerge(checkout) {
  return runScript(checkout.directory, 'merge-release-manifests.js', { GITHUB_SHA: checkout.commit });
}
