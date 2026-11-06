import assert from 'node:assert/strict';
import test from 'node:test';
import { createHash } from 'node:crypto';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { gzipSync } from 'node:zlib';
import { build } from 'esbuild';

const root = fileURLToPath(new URL('../', import.meta.url));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const modules = {};
for (const name of ['package-archive', 'package-release', 'package-promotion']) {
  const result = await build({ entryPoints: [join(root, 'scripts', `${name}.ts`)],
    bundle: true, write: false, platform: 'node', format: 'esm', metafile: true });
  const directory = mkdtempSync(join(tmpdir(), 'promotion-archive-module-'));
  try {
    const file = join(directory, `${name}.mjs`);
    writeFileSync(file, result.outputFiles[0].contents);
    Object.assign(modules, await import(pathToFileURL(file).href));
  } finally { rmSync(directory, { recursive: true, force: true }); }
  console.log(JSON.stringify({ module: name, bundle: hash(result.outputFiles[0].contents),
    inputs: Object.keys(result.metafile.inputs).map(path => ({ path, sha256: hash(readFileSync(path)) })) }));
}

function member(name, payload) {
  const header = Buffer.alloc(512);
  header.write(name);
  header.write('0000600\0', 100);
  header.write('0000000\0', 108);
  header.write('0000000\0', 116);
  header.write(`${payload.length.toString(8).padStart(11, '0')}\0`, 124);
  header.write('00000000000\0', 136);
  header.fill(32, 148, 156);
  header[156] = 48;
  header.write('ustar\0', 257);
  header.write('00', 263);
  const checksum = header.reduce((sum, byte) => sum + byte, 0);
  header.write(`${checksum.toString(8).padStart(6, '0')}\0 `, 148);
  return Buffer.concat([header, payload, Buffer.alloc((512 - payload.length % 512) % 512)]);
}

function archive(name, payloadBytes) {
  return gzipSync(Buffer.concat([
    member('package/package.json', Buffer.from(JSON.stringify({ name, version: '0.2.0' }))),
    member('package/synthetic.bin', Buffer.alloc(payloadBytes, 42)), Buffer.alloc(1024),
  ]), { level: 0 });
}

function save(path, bytes) {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  writeFileSync(path, bytes, { mode: 0o600, flag: 'wx' });
}
const source = { repository: 'synthetic/fixture', sha: 'a'.repeat(40),
  tree: 'b'.repeat(40), event: 'push', runId: '1234', runAttempt: '1' };
const targets = [['linux-x64', 'linux', 'x64', 'ubuntu-22.04'],
  ['macos-arm64', 'darwin', 'arm64', 'macos-15'], ['windows-x64', 'win32', 'x64', 'windows-2022']];

function assemble(directory, remoteArchive) {
  const input = join(directory, 'input'), reports = join(directory, 'reports');
  const candidate = join(directory, 'candidate');
  mkdirSync(join(input, 'tooling'), { recursive: true });
  const packages = ['framerize-remote', 'framerize'].map(name => {
    const data = name === 'framerize-remote' ? remoteArchive : archive(name, 16);
    const tarball = `${name}-0.2.0.tgz`;
    save(join(input, tarball), data);
    return { name, version: '0.2.0', tarball, bytes: data.length, sha256: hash(data) };
  });
  const producer = Buffer.from(JSON.stringify({ source, node: 'v22.23.4', npm: '10.9.9', packages }));
  save(join(input, 'manifest.json'), producer);
  const inventories = packages.map(({ bytes, ...entry }) => {
    const inventory = [...modules.readPackageArchive(join(input, entry.tarball))]
      .map(([path, data]) => ({ path, bytes: data.length, sha256: hash(data) }));
    return { ...entry, inventory, files: inventory.length };
  });
  for (const [target, platform, arch, runner] of targets) for (const node of ['22.12.0', '22.23.4', '24.13.0']) {
    const folder = join(reports, `package-coexistence-1234-report-${target}-node-${node}`);
    const log = Buffer.from('Synthetic only. No installed or native check executed.');
    const checker = { node: `v${node}`, npm: '10.9.9', platform, arch, packages: inventories };
    const report = { ...checker, target, runner, source,
      producer: { source, manifestSha256: hash(producer) }, installedLogSha256: hash(log) };
    save(join(folder, 'installed.log'), log);
    save(join(folder, 'checker.json'), JSON.stringify(checker));
    save(join(folder, 'report.json'), JSON.stringify(report));
  }
  const release = modules.assemblePackageRelease(input, reports, candidate, {
    ...source, tag: 'framerize-remote-v0.2.0', versions: { 'framerize-remote': '0.2.0', framerize: '0.2.0' },
  });
  assert.equal(release.publishable, false);
  const original = readFileSync(join(candidate, 'framerize-remote-0.2.0.tgz'));
  assert.deepEqual(original, remoteArchive);
  return candidate;
}

function approval(directory, candidate) {
  const evidence = Buffer.from('Synthetic fixture only. NOT release authorization or live qualification.');
  save(join(directory, 'framerize-remote-live.md'), evidence);
  const releaseSha256 = hash(readFileSync(join(candidate, 'release.json')));
  const approvalFile = join(directory, 'approval.json');
  save(approvalFile, JSON.stringify({ schemaVersion: 1, decision: 'approve-npm-publication',
    candidateRunId: source.runId, candidateManifestSha256: releaseSha256,
    sourceSha: source.sha, reviewedBy: 'synthetic-reviewer', evidenceFile: 'framerize-remote-live.md',
    evidenceSha256: hash(evidence) }));
  const expected = { repository: source.repository, runId: source.runId, releaseSha256,
    approvalCommit: 'c'.repeat(40), run: { id: 1234, run_attempt: 1, head_sha: source.sha,
      event: 'push', status: 'completed', conclusion: 'success', path: '.github/workflows/release.yml',
      repository: { full_name: source.repository }, head_repository: { full_name: source.repository } },
    environment: { protection_rules: [{ type: 'required_reviewers', reviewers: [{ type: 'User' }] }] } };
  return { approvalFile, expected };
}

const MiB = 1024 * 1024;
for (const payload of [16, 2 * MiB, 16 * MiB - 8192]) {
  test(`promotion preserves an assembled archive with ${payload} payload bytes`, { timeout: 5000 }, context => {
    const directory = mkdtempSync(join(tmpdir(), 'promotion-archive-case-'));
    context.after(() => rmSync(directory, { recursive: true, force: true }));
    const original = archive('framerize-remote', payload);
    assert.ok(original.length <= 16 * MiB);
    if (payload > MiB) assert.ok(original.length > 2 * MiB);
    const candidate = assemble(directory, original);
    const { approvalFile, expected } = approval(directory, candidate);
    const output = join(directory, 'promotion');
    let result, failure, promoted;
    try {
      result = modules.prepareRemotePromotion(candidate, approvalFile, output, expected);
      promoted = readFileSync(result.tarball);
    } catch (error) { failure = error; }
    context.diagnostic(JSON.stringify({ synthetic: true, archiveBytes: original.length,
      assembled: true, promoted: Boolean(result), error: failure?.message, outputExists: existsSync(output) }));
    rmSync(directory, { recursive: true, force: true });
    assert.equal(existsSync(directory), false);
    assert.ifError(failure);
    assert.deepEqual(promoted, original);
    assert.equal(result.sha256, hash(original));
    assert.equal(result.distTag, 'latest');
  });
}

test('promotion retains the 16 MiB archive ceiling', { timeout: 5000 }, context => {
  const directory = mkdtempSync(join(tmpdir(), 'promotion-archive-overflow-'));
  context.after(() => rmSync(directory, { recursive: true, force: true }));
  const candidate = assemble(directory, archive('framerize-remote', 16));
  const oversized = archive('framerize-remote', 16 * MiB);
  assert.ok(oversized.length > 16 * MiB);
  assert.throws(() => modules.readPackageArchive(oversized), /Invalid archive size/);
  const releaseFile = join(candidate, 'release.json');
  const release = JSON.parse(readFileSync(releaseFile, 'utf8'));
  const tarball = 'framerize-remote-0.2.0.tgz';
  const entry = release.evidence.find(value => value.file === tarball);
  entry.bytes = oversized.length;
  entry.sha256 = hash(oversized);
  writeFileSync(join(candidate, tarball), oversized);
  const releaseBytes = Buffer.from(JSON.stringify(release));
  writeFileSync(releaseFile, releaseBytes);
  writeFileSync(join(candidate, 'SHA256SUMS'),
    [...release.evidence, { file: 'release.json', sha256: hash(releaseBytes) }]
      .map(value => `${value.sha256}  ${value.file}\n`).join(''));
  const { approvalFile, expected } = approval(directory, candidate);
  const output = join(directory, 'promotion');
  assert.throws(() => modules.prepareRemotePromotion(candidate, approvalFile, output, expected),
    /Invalid promotion input/);
  assert.equal(existsSync(output), false);
  rmSync(directory, { recursive: true, force: true });
  assert.equal(existsSync(directory), false);
  context.diagnostic(JSON.stringify({ synthetic: true, archiveBytes: oversized.length,
    rejected: true, privateStateRemoved: true }));
});
