import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const source = fileURLToPath(new URL('../scripts/check.js', import.meta.url));
const sources = ['accounts.js', 'cli.js', 'config.js', 'errors.js', 'lifecycle.js', 'logger.js',
  'provider.js', 'server.js', 'transport.js'];

function fixture(t) {
  const root = mkdtempSync(path.join(tmpdir(), 'framerize-check-fixture-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const name of ['src', 'bin', 'scripts']) mkdirSync(path.join(root, name));
  copyFileSync(source, path.join(root, 'scripts', 'check.js'));
  for (const name of sources) writeFileSync(path.join(root, 'src', name), 'export const fixture = true;\n');
  writeFileSync(path.join(root, 'bin', 'framerize-server.js'), '#!/usr/bin/env node\n');
  const manifest = {
    name: 'framerize-server', private: true,
    files: ['bin', 'src', 'README.md', 'LICENSE'],
    exports: {
      '.': './src/server.js', './config': './src/config.js', './accounts': './src/accounts.js',
      './logger': './src/logger.js', './provider': './src/provider.js',
    },
    bin: { 'framerize-server': './bin/framerize-server.js' },
  };
  writeFileSync(path.join(root, 'package.json'), JSON.stringify(manifest));
  return { root, manifest, save: () => writeFileSync(path.join(root, 'package.json'), JSON.stringify(manifest)) };
}

function check(root) {
  return spawnSync(process.execPath, [path.join(root, 'scripts', 'check.js')], {
    cwd: root, encoding: 'utf8', timeout: 5_000,
  });
}

test('approved package identity and inventories pass static checking', (t) => {
  const checkout = fixture(t);
  const result = check(checkout.root);
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
});

test('public packages and extra package entries are rejected', (t) => {
  for (const [change, error] of [
    [{ private: false }, 'Server package must remain private'],
    [{ files: ['bin', 'src', 'README.md', 'LICENSE', 'castle'] }, 'Unexpected server package files'],
    [{ exports: { '.': './src/server.js' } }, 'Unexpected server package exports'],
    [{ bin: { 'framerize-server': './bin/framerize-server.js', extra: './bin/extra.js' } }, 'Unexpected server package executable'],
    [{ dependencies: { example: '1.0.0' } }, 'Unexpected runtime dependencies'],
    [{ optionalDependencies: { example: '1.0.0' } }, 'Unexpected runtime dependencies'],
    [{ peerDependencies: { example: '1.0.0' } }, 'Unexpected runtime dependencies'],
  ]) {
    const checkout = fixture(t);
    Object.assign(checkout.manifest, change);
    checkout.save();
    const result = check(checkout.root);
    assert.notEqual(result.status, 0);
    assert.match(`${result.stdout}\n${result.stderr}`, new RegExp(error));
  }
});

test('extra source and executable files are rejected without importing them', (t) => {
  for (const [directory, name, error] of [
    ['src', 'castle.js', 'Unexpected server source files'],
    ['bin', 'extra.js', 'Unexpected server executable files'],
  ]) {
    const checkout = fixture(t);
    writeFileSync(path.join(checkout.root, directory, name), 'throw new Error("must not import");\n');
    const result = check(checkout.root);
    assert.notEqual(result.status, 0);
    assert.match(`${result.stdout}\n${result.stderr}`, new RegExp(error));
    assert.doesNotMatch(`${result.stdout}\n${result.stderr}`, /must not import/);
  }
});

test('expanded Castle manifest is rejected as inert JSON', (t) => {
  const checkout = fixture(t);
  checkout.manifest.files.push('castle');
  checkout.manifest.exports['./castle'] = './src/castle.js';
  checkout.save();
  const result = check(checkout.root);
  assert.notEqual(result.status, 0);
  assert.match(`${result.stdout}\n${result.stderr}`, /Unexpected server package files/);
});
