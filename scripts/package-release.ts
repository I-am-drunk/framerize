import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { lstatSync, readFileSync, readdirSync, mkdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readPackageArchive } from './package-archive.js';

interface Source { repository: string; sha: string; tree: string; event: string; runId: string; runAttempt: string }
interface Package { name: string; version: string; tarball: string; sha256: string; bytes: number }
interface Producer { source: Source; node: string; npm: string; packages: Package[] }
interface InventoryEntry { path: string; sha256: string; bytes: number }
interface PackageReport extends Omit<Package, 'bytes'> { files: number; inventory: InventoryEntry[] }
interface Checker { node: string; npm: string; platform: string; arch: string; packages: PackageReport[] }
interface Consumer extends Checker {
  source: Source;
  target: string;
  runner: string;
  producer: { manifestSha256: string; source: Source };
  installedLogSha256: string;
}
export interface ReleaseContext extends Source { tag: string; versions: Record<string, string> }
const digest = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex');
const nodes = ['22.12.0', '22.23.4', '24.13.0'];
const targets = [
  { target: 'linux-x64', platform: 'linux', arch: 'x64', runner: 'ubuntu-22.04' },
  { target: 'macos-arm64', platform: 'darwin', arch: 'arm64', runner: 'macos-15' },
  { target: 'windows-x64', platform: 'win32', arch: 'x64', runner: 'windows-2022' },
];
function bytes(file: string, limit = 16 * 1024 * 1024): Buffer {
  const info = lstatSync(file);
  assert.ok(info.isFile() && info.size <= limit, 'Invalid release input');
  return readFileSync(file);
}
function sourceMatches(actual: Source, expected: Source): void {
  for (const key of ['repository', 'sha', 'tree', 'event', 'runId'] as const) {
    assert.equal(actual[key], expected[key], `Release source ${key} differs`);
  }
  assert.match(actual.runAttempt, /^[1-9]\d*$/);
  assert.ok(Number.isSafeInteger(Number(actual.runAttempt))
    && Number(actual.runAttempt) <= Number(expected.runAttempt), 'Invalid run attempt');
}

export function assemblePackageRelease(input: string, reports: string, output: string, expected: ReleaseContext) {
  const manifestFile = join(input, 'manifest.json');
  const manifestBytes = bytes(manifestFile, 2 * 1024 * 1024);
  const manifestHash = digest(manifestBytes);
  const producer = JSON.parse(manifestBytes.toString('utf8')) as Producer;
  sourceMatches(producer.source, expected);
  assert.equal(expected.event, 'push', 'Release requires a tag push');
  assert.equal(expected.tag, `framerize-remote-v${expected.versions['framerize-remote']}`, 'Release tag differs');
  assert.equal(producer.node, 'v22.23.4');
  assert.match(producer.npm, /^\d+\.\d+\.\d+$/);
  assert.deepEqual(producer.packages.map(entry => entry.name), ['framerize-remote', 'framerize']);
  assert.deepEqual(readdirSync(input).sort(), ['manifest.json', 'tooling',
    ...producer.packages.map(entry => entry.tarball)].sort());
  const snapshots = new Map<string, Buffer>();
  const inventories = new Map<string, InventoryEntry[]>();
  for (const entry of producer.packages) {
    assert.equal(entry.version, expected.versions[entry.name]);
    assert.match(entry.version, /^\d+\.\d+\.\d+(?:-[\w.-]+)?(?:\+[\w.-]+)?$/);
    assert.equal(entry.tarball, `${entry.name}-${entry.version}.tgz`);
    const snapshot = bytes(join(input, entry.tarball));
    assert.equal(snapshot.length, entry.bytes); assert.equal(digest(snapshot), entry.sha256);
    snapshots.set(entry.name, snapshot);
    const archive = readPackageArchive(snapshot);
    const manifest = JSON.parse(archive.get('package/package.json')!.toString('utf8')) as Package;
    assert.equal(manifest.name, entry.name); assert.equal(manifest.version, entry.version);
    inventories.set(entry.name, [...archive].map(([path, payload]) => ({ path,
      bytes: payload.length, sha256: digest(payload) })));
  }
  const matrix = targets.flatMap(target => nodes.map(node => ({ ...target, node,
    folder: `package-coexistence-${expected.runId}-report-${target.target}-node-${node}` })));
  assert.deepEqual(readdirSync(reports).sort(), matrix.map(entry => entry.folder).sort(), 'Incomplete release matrix');
  const verified = new Map<string, Buffer>();
  for (const cell of matrix) {
    const folder = join(reports, cell.folder);
    assert.ok(lstatSync(folder).isDirectory(), 'Invalid report directory');
    assert.deepEqual(readdirSync(folder).sort(), ['checker.json', 'installed.log', 'report.json']);
    const reportBytes = bytes(join(folder, 'report.json'), 2 * 1024 * 1024);
    const checkerBytes = bytes(join(folder, 'checker.json'), 2 * 1024 * 1024);
    const installedBytes = bytes(join(folder, 'installed.log'));
    const report = JSON.parse(reportBytes.toString('utf8')) as Consumer;
    const checker = JSON.parse(checkerBytes.toString('utf8')) as Checker;
    sourceMatches(report.source, expected);
    assert.ok(Number(report.source.runAttempt) >= Number(producer.source.runAttempt));
    assert.deepEqual(report.producer.source, producer.source);
    assert.equal(report.producer.manifestSha256, manifestHash);
    assert.equal(report.node, `v${cell.node}`); assert.equal(checker.node, report.node);
    assert.equal(report.target, cell.target); assert.equal(report.runner, cell.runner);
    assert.equal(report.platform, cell.platform); assert.equal(report.arch, cell.arch);
    assert.equal(checker.platform, cell.platform); assert.equal(checker.arch, cell.arch);
    assert.match(report.npm, /^\d+\.\d+\.\d+$/); assert.equal(checker.npm, report.npm);
    assert.deepEqual(report.packages, checker.packages);
    assert.deepEqual(report.packages, producer.packages.map(entry => ({ name: entry.name,
      version: entry.version, tarball: entry.tarball, sha256: entry.sha256,
      files: inventories.get(entry.name)!.length, inventory: inventories.get(entry.name) })),
    'Release package inventory differs');
    assert.equal(digest(installedBytes), report.installedLogSha256);
    for (const [file, data] of [['report.json', reportBytes], ['checker.json', checkerBytes],
      ['installed.log', installedBytes]] as const) {
      verified.set(`reports/${cell.target}-node-${cell.node}/${file}`, data);
    }
  }
  const remote = producer.packages[0]!;
  verified.set(remote.tarball, snapshots.get(remote.name)!);
  verified.set('producer.json', manifestBytes);
  const release = { name: remote.name, version: remote.version, tag: expected.tag,
    source: producer.source, assembledAttempt: expected.runAttempt,
    publishable: false, pendingGate: 'live_browserless_qualification',
    packages: producer.packages, node: producer.node, npm: producer.npm,
    evidence: [...verified].map(([file, data]) => ({ file, bytes: data.length, sha256: digest(data) })) };
  verified.set('release.json', Buffer.from(JSON.stringify(release, null, 2) + '\n'));
  mkdirSync(output);
  for (const [file, data] of verified) {
    const target = join(output, file);
    mkdirSync(dirname(target), { recursive: true });
    writeFileSync(target, data, { flag: 'wx' });
  }
  writeFileSync(join(output, 'SHA256SUMS'), [...verified].map(([file, data]) =>
    `${digest(data)}  ${file}\n`).join(''), { flag: 'wx' });
  return release;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    assert.equal(process.argv.length, 5);
    const env = process.env;
    for (const key of ['GITHUB_SHA', 'GITHUB_REPOSITORY', 'GITHUB_RUN_ID',
      'GITHUB_RUN_ATTEMPT', 'GITHUB_REF', 'GITHUB_EVENT_NAME']) assert.ok(env[key]);
    const git = (argument: string) => execFileSync('git', ['rev-parse', argument], { encoding: 'utf8' }).trim();
    assert.equal(git('HEAD'), env.GITHUB_SHA);
    const versions = Object.fromEntries(['framerize-remote', 'framerize'].map(name =>
      [name, (JSON.parse(bytes(join(name, 'package.json')).toString()) as { version: string }).version]));
    const result = assemblePackageRelease(process.argv[2]!, process.argv[3]!, process.argv[4]!, {
      repository: env.GITHUB_REPOSITORY!, sha: env.GITHUB_SHA!, tree: git('HEAD^{tree}'),
      event: env.GITHUB_EVENT_NAME!, runId: env.GITHUB_RUN_ID!, runAttempt: env.GITHUB_RUN_ATTEMPT!,
      tag: env.GITHUB_REF!.replace(/^refs\/tags\//, ''), versions,
    });
    console.log(JSON.stringify(result));
  } catch {
    console.error('Package release qualification failed'); process.exitCode = 1;
  }
}
