import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { gzipSync } from 'node:zlib';
import test from 'node:test';
import { build } from 'esbuild';

const root = fileURLToPath(new URL('../', import.meta.url));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const modules = {};
for (const name of ['package-release', 'package-promotion', 'package-archive']) {
  const result = await build({ entryPoints: [join(root, 'scripts', `${name}.ts`)],
    bundle: true, write: false, platform: 'node', format: 'esm', metafile: true });
  const bytes = result.outputFiles[0].contents;
  const directory = mkdtempSync(join(tmpdir(), 'framerize-promotion-module-'));
  try {
    const file = join(directory, `${name}.mjs`);
    writeFileSync(file, bytes, { mode: 0o600, flag: 'wx' });
    Object.assign(modules, await import(pathToFileURL(file).href));
  } finally { rmSync(directory, { recursive: true, force: true }); }
  console.log(JSON.stringify({ module: name, bundleSha256: hash(bytes),
    inputs: Object.keys(result.metafile.inputs).map(path => ({ path, sha256: hash(readFileSync(path)) })) }));
}

function save(path, value) {
  mkdirSync(dirname(path), { recursive: true, mode: 0o700 });
  writeFileSync(path, value, { mode: 0o600, flag: 'wx' });
}

function archive(name, version) {
  const payload = Buffer.from(JSON.stringify({ name, version }));
  const header = Buffer.alloc(512);
  header.write('package/package.json');
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
  return gzipSync(Buffer.concat([header, payload,
    Buffer.alloc((512 - payload.length % 512) % 512), Buffer.alloc(1024)]));
}

const platforms = [['linux-x64', 'linux', 'x64', 'ubuntu-22.04'],
  ['macos-arm64', 'darwin', 'arm64', 'macos-15'], ['windows-x64', 'win32', 'x64', 'windows-2022']];
const nodes = ['22.12.0', '22.23.4', '24.13.0'];
const syntheticSource = { repository: 'synthetic/fixture', sha: 'a'.repeat(40),
  tree: 'b'.repeat(40), event: 'push', runId: '1234', runAttempt: '1' };

function fixture(directory, version) {
  const input = join(directory, 'input');
  const reports = join(directory, 'reports');
  const candidate = join(directory, 'candidate');
  mkdirSync(join(input, 'tooling'), { recursive: true });
  const packages = [['framerize-remote', version], ['framerize', '0.1.0']].map(([name, value]) => {
    const bytes = archive(name, value);
    const tarball = `${name}-${value}.tgz`;
    save(join(input, tarball), bytes);
    return { name, version: value, tarball, bytes: bytes.length, sha256: hash(bytes) };
  });
  const producer = { source: syntheticSource, node: 'v22.23.4', npm: '10.9.9', packages };
  const producerBytes = Buffer.from(JSON.stringify(producer));
  save(join(input, 'manifest.json'), producerBytes);
  const inventories = packages.map(({ bytes, ...entry }) => {
    const inventory = [...modules.readPackageArchive(join(input, entry.tarball))]
      .map(([path, data]) => ({ path, bytes: data.length, sha256: hash(data) }));
    return { ...entry, inventory, files: inventory.length };
  });
  for (const [target, platform, arch, runner] of platforms) for (const node of nodes) {
    const folder = join(reports, `package-coexistence-1234-report-${target}-node-${node}`);
    const log = Buffer.from('Synthetic receipt, no platform or installed test executed.');
    const checker = { node: `v${node}`, npm: '10.9.9', platform, arch, packages: inventories };
    const report = { ...checker, target, runner, source: syntheticSource,
      producer: { source: syntheticSource, manifestSha256: hash(producerBytes) }, installedLogSha256: hash(log) };
    save(join(folder, 'installed.log'), log);
    save(join(folder, 'checker.json'), JSON.stringify(checker));
    save(join(folder, 'report.json'), JSON.stringify(report));
  }
  const release = modules.assemblePackageRelease(input, reports, candidate, {
    ...syntheticSource, tag: `framerize-remote-v${version}`,
    versions: { 'framerize-remote': version, framerize: '0.1.0' },
  });
  assert.equal(release.version, version);
  assert.equal(release.publishable, false);
  const tarball = packages[0].tarball;
  const original = readFileSync(join(input, tarball));
  assert.deepEqual(readFileSync(join(candidate, tarball)), original);
  const releaseSha256 = hash(readFileSync(join(candidate, 'release.json')));
  const evidence = Buffer.from('Synthetic approval fixture. NOT live qualification or publication authority.');
  save(join(directory, 'framerize-remote-live.md'), evidence);
  const approvalFile = join(directory, 'approval.json');
  save(approvalFile, JSON.stringify({ schemaVersion: 1, decision: 'approve-npm-publication',
    candidateRunId: '1234', candidateManifestSha256: releaseSha256, sourceSha: syntheticSource.sha,
    reviewedBy: 'synthetic-reviewer', evidenceFile: 'framerize-remote-live.md', evidenceSha256: hash(evidence) }));
  const output = join(directory, 'promotion');
  const expected = { repository: syntheticSource.repository, runId: '1234', releaseSha256,
    approvalCommit: 'c'.repeat(40), run: { id: 1234, run_attempt: 1, head_sha: syntheticSource.sha,
      event: 'push', status: 'completed', conclusion: 'success', path: '.github/workflows/release.yml',
      repository: { full_name: syntheticSource.repository },
      head_repository: { full_name: syntheticSource.repository } },
    environment: { protection_rules: [{ type: 'required_reviewers', reviewers: [{ type: 'User' }] }] } };
  return { candidate, approvalFile, output, expected, original };
}

for (const [version, distTag] of [['0.2.0', 'latest'], ['0.2.0-rc.1', 'next'],
  ['0.2.0+build.7', 'latest'], ['0.2.0+build-beta', 'latest'], ['0.2.0-rc.1+build.7', 'next']]) {
  test(`synthetic release-to-promotion preserves ${version}`, { timeout: 5000 }, context => {
    const directory = mkdtempSync(join(tmpdir(), 'framerize-version-case-'));
    context.after(() => rmSync(directory, { recursive: true, force: true }));
    const state = fixture(directory, version);
    let result, failure, promotedBytes;
    try {
      result = modules.prepareRemotePromotion(state.candidate, state.approvalFile, state.output, state.expected);
      promotedBytes = readFileSync(result.tarball);
    } catch (error) { failure = error; }
    context.diagnostic(JSON.stringify({ synthetic: true, version, assembled: true,
      promoted: result !== undefined, error: failure?.message, outputExists: existsSync(state.output) }));
    rmSync(directory, { recursive: true, force: true });
    assert.equal(existsSync(directory), false, 'Private release fixtures must be removed');
    context.diagnostic('Private release fixtures removed; no processes or sockets were created');
    assert.ifError(failure);
    assert.equal(result.version, version);
    assert.equal(result.distTag, distTag);
    assert.equal(result.sha256, hash(state.original));
    assert.deepEqual(promotedBytes, state.original);
  });
}
