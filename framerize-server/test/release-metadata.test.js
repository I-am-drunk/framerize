import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { artifactNames, currentTarget, targets, verifySums } from '../scripts/release-artifacts.js';
import { PACKAGE, assertFailure, refreshTarget, runMerge, runScript, seedBuild, seedRelease } from './helpers/release-fixtures.js';

test('artifact names and current host target match the supported matrix', () => {
  assert.deepEqual(Object.keys(targets), ['linux-x64', 'macos-arm64', 'windows-x64']);
  assert.deepEqual(artifactNames('linux-x64'), ['framerize-server', 'framerize-server-linux-x64.tar.gz']);
  assert.deepEqual(artifactNames('windows-x64'), ['framerize-server.exe', 'framerize-server-windows-x64.zip']);
  assert.ok(Object.hasOwn(targets, currentTarget()));
  assert.throws(() => artifactNames('freebsd-x64'), /Unsupported executable target/);
});

test('release metadata records exactly five verified artifacts', (t) => {
  const checkout = seedBuild(t);
  const result = runScript(checkout.directory, 'release-metadata.js', { GITHUB_SHA: checkout.commit });
  assert.equal(result.status, 0, result.stderr);
  const metadata = JSON.parse(readFileSync(path.join(checkout.dist, 'release.json'), 'utf8'));
  assert.deepEqual(Object.keys(metadata.artifacts).sort(), [
    'LICENSE', 'NODE-LICENSE', PACKAGE, checkout.archive, checkout.executable,
  ].sort());
  assert.equal(metadata.commit, checkout.commit);
  assert.equal(metadata.target, currentTarget());
  assert.equal(metadata.node, process.version);
  verifySums(checkout.dist, metadata.artifacts);
});

test('release metadata rejects a stale executable build', (t) => {
  const checkout = seedBuild(t);
  const file = path.join(checkout.dist, 'build.json');
  const build = JSON.parse(readFileSync(file, 'utf8'));
  writeFileSync(file, JSON.stringify({ ...build, commit: '0'.repeat(40) }));
  assertFailure(runScript(checkout.directory, 'release-metadata.js', { GITHUB_SHA: checkout.commit }), 'Stale executable build');
});

test('aggregate accepts three exact platform manifests and shared files', (t) => {
  const checkout = seedRelease(t);
  const result = runMerge(checkout);
  assert.equal(result.status, 0, result.stderr);
  const output = path.join(checkout.dist, 'qualified');
  const metadata = JSON.parse(readFileSync(path.join(output, 'release.json'), 'utf8'));
  assert.equal(metadata.builds.length, 3);
  assert.deepEqual(Object.keys(metadata.artifacts).sort(), [
    PACKAGE, 'LICENSE', 'NODE-LICENSE', 'framerize-server-linux-x64.tar.gz',
    'framerize-server-macos-arm64.tar.gz', 'framerize-server-windows-x64.zip',
  ].sort());
  verifySums(output, metadata.artifacts);
});

test('aggregate rejects duplicate checksum rows', (t) => {
  const checkout = seedRelease(t);
  const target = checkout.targets['linux-x64'];
  const file = path.join(target.dist, 'SHA256SUMS');
  const lines = readFileSync(file, 'utf8').trim().split('\n');
  writeFileSync(file, `${[...lines, lines[0]].join('\n')}\n`);
  assertFailure(runMerge(checkout), 'Checksum manifest mismatch');
});

test('aggregate rejects a replaced root shared tarball', (t) => {
  const checkout = seedRelease(t);
  writeFileSync(path.join(checkout.dist, PACKAGE), 'different package\n');
  assertFailure(runMerge(checkout), `Shared artifact mismatch: ${PACKAGE}`);
});

test('aggregate rejects a substituted project LICENSE in every manifest', (t) => {
  const checkout = seedRelease(t);
  for (const target of Object.values(checkout.targets)) {
    writeFileSync(path.join(target.dist, 'LICENSE'), 'Consistent substituted license\n');
    refreshTarget(target);
  }
  assertFailure(runMerge(checkout), 'Project license differs from checkout');
});

test('aggregate rejects empty Node notices and CRLF project notices', (t) => {
  for (const [name, content, error] of [
    ['NODE-LICENSE', ' \n', 'License must be nonempty canonical LF text'],
    ['LICENSE', 'Project license\r\n', 'License must be nonempty canonical LF text'],
  ]) {
    const checkout = seedRelease(t);
    const target = checkout.targets['macos-arm64'];
    writeFileSync(path.join(target.dist, name), content);
    refreshTarget(target);
    assertFailure(runMerge(checkout), error);
  }
});

test('aggregate rejects unsupported or inconsistent Node versions', (t) => {
  for (const node of ['v22.11.0', 'v24.0.0']) {
    const checkout = seedRelease(t);
    const target = checkout.targets['windows-x64'];
    const file = path.join(target.dist, 'release.json');
    const metadata = JSON.parse(readFileSync(file, 'utf8'));
    writeFileSync(file, JSON.stringify({ ...metadata, node }));
    assertFailure(runMerge(checkout), 'Release Node versions differ or are unsupported');
  }
});

test('aggregate rejects missing artifact declarations', (t) => {
  const checkout = seedRelease(t);
  const file = path.join(checkout.targets['linux-x64'].dist, 'release.json');
  const metadata = JSON.parse(readFileSync(file, 'utf8'));
  delete metadata.artifacts['NODE-LICENSE'];
  writeFileSync(file, JSON.stringify(metadata));
  assertFailure(runMerge(checkout), 'Unexpected build artifacts for linux-x64');
});

test('aggregate rejects a mismatched target and source commit', (t) => {
  for (const change of [{ target: 'linux-x64' }, { commit: '0'.repeat(40) }]) {
    const checkout = seedRelease(t);
    const file = path.join(checkout.targets['macos-arm64'].dist, 'release.json');
    const metadata = JSON.parse(readFileSync(file, 'utf8'));
    writeFileSync(file, JSON.stringify({ ...metadata, ...change }));
    assertFailure(runMerge(checkout), 'Release identity mismatch for macos-arm64');
  }
});

test('metadata rejects bytes changed without refreshing the artifact hash', (t) => {
  const checkout = seedRelease(t);
  const target = checkout.targets['linux-x64'];
  writeFileSync(path.join(target.dist, target.executable), 'mutated executable\n');
  assertFailure(runMerge(checkout), 'Artifact digest mismatch');
});

test('source identity rejects dirty tracked server and workflow files', (t) => {
  for (const file of ['package.json', '../.github/workflows/server-ci.yml']) {
    const checkout = seedRelease(t);
    writeFileSync(path.join(checkout.directory, file), file.endsWith('.json')
      ? '{"name":"framerize-server","version":"0.1.0","private":true,"dirty":true}'
      : 'name: dirty\n');
    assertFailure(runMerge(checkout), 'Packaging requires a clean committed server checkout');
  }
});

test('source identity rejects uncommitted shared build inputs', async (context) => {
  const files = ['framerize-core/src/instance.ts', 'framerize-core/package.json',
    'framerize-core/tsconfig.json', 'scripts/build-server.ts', 'package.json',
    'scripts/server-native-bundle.ts', 'scripts/package-archive.ts',
    'package-lock.json', 'tsconfig.base.json', 'tsconfig.tools.json', 'tsconfig.server-tools.json'];
  for (const file of files) await context.test(file, (fixtureContext) => {
    const checkout = seedRelease(fixtureContext);
    const destination = path.resolve(checkout.directory, '..', file);
    mkdirSync(path.dirname(destination), { recursive: true });
    writeFileSync(destination, 'untracked shared build input\n');
    assertFailure(runMerge(checkout), 'Packaging requires a clean committed server checkout');
    checkout.git(['add', file]);
    checkout.git(['commit', '-qm', 'shared build fixture']);
    checkout.commit = checkout.git(['rev-parse', 'HEAD']).trim();
    writeFileSync(destination, 'changed shared build input\n');
    assertFailure(runMerge(checkout), 'Packaging requires a clean committed server checkout');
    checkout.git(['add', file]);
    assertFailure(runMerge(checkout), 'Packaging requires a clean committed server checkout');
  });
});

test('release metadata rejects a package changed after building', context => {
  const checkout = seedBuild(context);
  writeFileSync(path.join(checkout.directory, PACKAGE), 'replacement archive');
  assertFailure(runScript(checkout.directory, 'release-metadata.js'), 'Stale executable build');
});

test('native packaging rejects changed package bytes before producing an archive', context => {
  const checkout = seedBuild(context);
  writeFileSync(path.join(checkout.directory, PACKAGE), 'replacement archive');
  assertFailure(runScript(checkout.directory, 'package-binary.js'), 'Stale executable build');
});

test('aggregate rejects a native build bound to a different package', context => {
  const checkout = seedRelease(context);
  const file = path.join(checkout.targets['linux-x64'].dist, 'release.json');
  const metadata = JSON.parse(readFileSync(file, 'utf8'));
  writeFileSync(file, JSON.stringify({ ...metadata, packageSha256: '0'.repeat(64) }));
  assertFailure(runMerge(checkout), 'Native build package binding mismatch');
});
