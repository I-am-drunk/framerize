import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { packageFixture, packageTarball, runPackageSmoke, assertPackageFailure, mutateTarball } from './helpers/package-fixtures.js';

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
  mutateTarball(t, checkout.directory, 'src/config.js', 'export const loadConfig = (values) => ({...values, changed:true});\n');
  assertPackageFailure(runPackageSmoke(checkout.directory, true), 'Installed source differs: src/config.js');
});

test('downloaded package smoke rejects a changed launcher body', (t) => {
  const checkout = packageFixture(t);
  packageTarball(checkout.directory);
  mutateTarball(t, checkout.directory, 'bin/framerize-server.js', Buffer.from('#!/usr/bin/env node\nconsole.log("changed");\r\n'));
  assertPackageFailure(runPackageSmoke(checkout.directory, true), 'Installed source differs: bin/framerize-server.js');
});

test('downloaded package smoke rejects CRLF conversion outside the launcher', (t) => {
  const checkout = packageFixture(t);
  packageTarball(checkout.directory);
  mutateTarball(t, checkout.directory, 'src/config.js', 'export const loadConfig = (values) => values;\r\n');
  assertPackageFailure(runPackageSmoke(checkout.directory, true), 'Installed source differs: src/config.js');
});
