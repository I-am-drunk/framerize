import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { packageFixture, packageTarball, runPackageSmoke, assertPackageFailure, mutateTarball } from './helpers/package-fixtures.js';

test('downloaded package smoke rejects a substituted core bundle', (t) => {
  const checkout = packageFixture(t);
  packageTarball(checkout.directory);
  mutateTarball(t, checkout.directory, 'dist/core.js', 'export const replaced = true;\n');
  assertPackageFailure(runPackageSmoke(checkout.directory, true), 'Installed source differs: dist/core.js');
});

for (const downloaded of [false, true]) {
  test(`${downloaded ? 'downloaded' : 'local'} package smoke isolates hostile operator environment`, (t) => {
    const checkout = packageFixture(t, { environmentAware: true });
    const env = { FRAMERIZE_SERVER_HOST: '0.0.0.0', FRAMERIZE_SERVER_PORT: '9999',
      FRAMERIZE_SERVER_UPSTREAM: 'https://inference.example.test' };
    if (downloaded) packageTarball(checkout.directory);
    const result = runPackageSmoke(checkout.directory, downloaded, env);
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
    const script = path.join(checkout.directory, 'scripts', 'package-smoke.js');
    const original = readFileSync(script, 'utf8');
    const regressed = original.replace("host:'127.0.0.1',port:0,upstream:'',", 'port:0,');
    assert.notEqual(regressed, original);
    writeFileSync(script, regressed);
    assertPackageFailure(runPackageSmoke(checkout.directory, downloaded, env),
      'Unsafe fixture configuration before listen');
  });
}

test('direct invocation rejects missing npm context without leaking temp directories', (t) => {
  const root = mkdtempSync(path.join(tmpdir(), 'framerize-smoke-context-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const env = { ...process.env, TMPDIR: root, TEMP: root, TMP: root };
  delete env.npm_execpath;
  const script = fileURLToPath(new URL('../scripts/package-smoke.js', import.meta.url));
  const result = spawnSync(process.execPath, [script], { env, encoding: 'utf8', timeout: 5_000 });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /Run this check with npm run pack:check/);
  assert.deepEqual(readdirSync(root), []);
});

test('local package smoke accepts only npm shebang CRLF normalization', (t) => {
  const checkout = packageFixture(t);
  const result = runPackageSmoke(checkout.directory);
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
});

test('downloaded LF tarball passes from a CRLF checkout', (t) => {
  const checkout = packageFixture(t);
  const launcher = path.join(checkout.directory, 'bin', 'framerize-server.js');
  writeFileSync(launcher, readFileSync(launcher, 'utf8').replaceAll('\r\n', '\n'));
  checkout.git(['add', 'framerize-server/bin/framerize-server.js']);
  checkout.git(['commit', '-qm', 'canonical LF package']);
  packageTarball(checkout.directory);
  writeFileSync(launcher, Buffer.from('#!/usr/bin/env node\r\nconsole.log("body");\r\n'));
  const result = runPackageSmoke(checkout.directory, true);
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
});

test('downloaded package smoke rejects bytes changed after commit', (t) => {
  const checkout = packageFixture(t);
  packageTarball(checkout.directory);
  mutateTarball(t, checkout.directory, 'dist/server.js', 'export const createServer = () => ({changed:true});\n');
  assertPackageFailure(runPackageSmoke(checkout.directory, true), 'Installed source differs: dist/server.js');
});

for (const name of ['config.js', 'config.d.ts', 'accounts.js', 'accounts.d.ts',
  'cli.js', 'cli.d.ts', 'logger.js', 'logger.d.ts', 'server.d.ts',
  'provider.js', 'provider.d.ts', 'core.d.ts',
  ...['backend', 'http-policy', 'instance', 'instance-options', 'readiness'].map((name) => `core-types/${name}.d.ts`)]) {
  test(`downloaded package smoke rejects substituted compiled ${name}`, (t) => {
    const checkout = packageFixture(t);
    packageTarball(checkout.directory);
    mutateTarball(t, checkout.directory, `dist/${name}`, 'export const replaced = true;\n');
    assertPackageFailure(runPackageSmoke(checkout.directory, true), `Installed source differs: dist/${name}`);
  });
}

test('downloaded package smoke rejects a changed launcher body', (t) => {
  const checkout = packageFixture(t);
  packageTarball(checkout.directory);
  mutateTarball(t, checkout.directory, 'bin/framerize-server.js', Buffer.from('#!/usr/bin/env node\nconsole.log("changed");\r\n'));
  assertPackageFailure(runPackageSmoke(checkout.directory, true), 'Installed source differs: bin/framerize-server.js');
});

test('downloaded package smoke rejects CRLF conversion outside the launcher', (t) => {
  const checkout = packageFixture(t);
  packageTarball(checkout.directory);
  const source = readFileSync(path.join(checkout.directory, 'dist/server.js'), 'utf8');
  mutateTarball(t, checkout.directory, 'dist/server.js', source.replaceAll('\n', '\r\n'));
  assertPackageFailure(runPackageSmoke(checkout.directory, true), 'Installed source differs: dist/server.js');
});

test('downloaded package smoke rejects optional dependencies in the manifest', (t) => {
  const checkout = packageFixture(t);
  packageTarball(checkout.directory);
  const manifest = JSON.parse(readFileSync(path.join(checkout.directory, 'package.json'), 'utf8'));
  manifest.optionalDependencies = { 'framerize-inert-missing-fixture': '0.0.0' };
  mutateTarball(t, checkout.directory, 'package.json', JSON.stringify(manifest));
  assertPackageFailure(runPackageSmoke(checkout.directory, true), 'Unexpected package dependencies');
});

test('downloaded package smoke rejects optional peer dependencies in the manifest', (t) => {
  const checkout = packageFixture(t);
  packageTarball(checkout.directory);
  const manifest = JSON.parse(readFileSync(path.join(checkout.directory, 'package.json'), 'utf8'));
  manifest.peerDependencies = { 'framerize-inert-missing-fixture': '0.0.0' };
  manifest.peerDependenciesMeta = { 'framerize-inert-missing-fixture': { optional: true } };
  mutateTarball(t, checkout.directory, 'package.json', JSON.stringify(manifest));
  assertPackageFailure(runPackageSmoke(checkout.directory, true), 'Unexpected package dependencies');
});

test('downloaded package smoke rejects an extra export', (t) => {
  const checkout = packageFixture(t);
  packageTarball(checkout.directory);
  const manifest = JSON.parse(readFileSync(path.join(checkout.directory, 'package.json'), 'utf8'));
  manifest.exports['./unexpected'] = './src/config.js';
  mutateTarball(t, checkout.directory, 'package.json', JSON.stringify(manifest));
  assertPackageFailure(runPackageSmoke(checkout.directory, true), 'Installed package metadata differs');
});

test('downloaded package smoke rejects an extra executable alias', (t) => {
  const checkout = packageFixture(t);
  packageTarball(checkout.directory);
  const manifest = JSON.parse(readFileSync(path.join(checkout.directory, 'package.json'), 'utf8'));
  manifest.bin['unexpected-cli'] = './bin/framerize-server.js';
  mutateTarball(t, checkout.directory, 'package.json', JSON.stringify(manifest));
  assertPackageFailure(runPackageSmoke(checkout.directory, true), 'Installed package metadata differs');
});

test('downloaded package smoke rejects an altered files declaration', (t) => {
  const checkout = packageFixture(t);
  packageTarball(checkout.directory);
  const manifest = JSON.parse(readFileSync(path.join(checkout.directory, 'package.json'), 'utf8'));
  manifest.files.push('unexpected');
  mutateTarball(t, checkout.directory, 'package.json', JSON.stringify(manifest));
  assertPackageFailure(runPackageSmoke(checkout.directory, true), 'Installed package metadata differs');
});

test('downloaded package smoke rejects an unexpected installed source file', (t) => {
  const checkout = packageFixture(t);
  packageTarball(checkout.directory);
  mutateTarball(t, checkout.directory, 'dist/unexpected.js', 'export const inert = true;\n');
  assertPackageFailure(runPackageSmoke(checkout.directory, true), 'Unexpected installed package files');
});
