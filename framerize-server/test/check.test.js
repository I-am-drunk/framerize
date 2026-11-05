import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const source = fileURLToPath(new URL('../scripts/check.js', import.meta.url));
const compatibility = ['completed-reply', 'completed-stream', 'events', 'messages-stream', 'responses-stream'];
const setupModules = ['startup-access', 'private-network', 'qr-code', 'settings', 'setup',
  'service-units', 'service-control', 'tailscale', 'browser-login', 'account-cli', 'account-store'];
const notices = ['README.md', 'LICENSE', 'T3-LICENSE', 'NAYUKI-LICENSE', 'T3-ATTRIBUTION.md'];
const sources = ['account-backend.ts', 'accounts.ts', 'cli.ts', 'config.ts', 'errors.ts', 'framer-auth.ts', 'framer-backend.ts',
  'framer-request.ts', 'logger.ts', 'provider.ts', 'server.ts', 'backend.ts', 'castle-loader.ts', 'castle.ts',
  ...setupModules.map(name => `${name}.ts`),
  ...compatibility.map(name => `compat/${name}.ts`)];
const generated = ['core.js', 'core.d.ts', 'accounts.js', 'accounts.d.ts', 'config.js', 'config.d.ts',
  ...setupModules.flatMap(name => [`${name}.js`, `${name}.d.ts`]),
  'backend.js', 'backend.d.ts', 'castle.js',
  'castle-loader.js', 'castle-loader.d.ts',
  'account-backend.js', 'account-backend.d.ts',
  'errors.js', 'errors.d.ts', 'provider.js', 'provider.d.ts',
  'framer-auth.js', 'framer-auth.d.ts',
  'framer-backend.js', 'framer-backend.d.ts', 'framer-request.js', 'framer-request.d.ts',
  ...compatibility.flatMap(name => [`compat/${name}.js`, `compat/${name}.d.ts`]),
  'cli.js', 'cli.d.ts', 'logger.js', 'logger.d.ts', 'server.js', 'server.d.ts',
  ...['backend', 'http-policy', 'instance', 'instance-options', 'readiness'].map((name) => `core-types/${name}.d.ts`)];

function fixture(t) {
  const root = mkdtempSync(path.join(tmpdir(), 'framerize-check-fixture-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const name of ['src', 'bin', 'scripts', 'dist']) mkdirSync(path.join(root, name));
  mkdirSync(path.join(root, 'dist/core-types'));
  mkdirSync(path.join(root, 'src/compat'));
  mkdirSync(path.join(root, 'dist/compat'));
  for (const name of generated) writeFileSync(path.join(root, 'dist', name), 'export const fixture = true;\n');
  copyFileSync(source, path.join(root, 'scripts', 'check.js'));
  for (const name of sources) writeFileSync(path.join(root, 'src', name), 'export const fixture = true;\n');
  writeFileSync(path.join(root, 'bin', 'framerize-server.js'), '#!/usr/bin/env node\n');
  const manifest = {
    name: 'framerize', type: 'module',
    files: ['bin', ...generated.map((name) => `dist/${name}`), ...notices],
    exports: {
      '.': './dist/server.js', './config': './dist/config.js', './accounts': './dist/accounts.js',
      './logger': './dist/logger.js', './provider': './dist/provider.js',
      './framer-auth': './dist/framer-auth.js',
      './framer-backend': './dist/framer-backend.js',
      './account-backend': './dist/account-backend.js',
      './startup-access': './dist/startup-access.js',
    },
    bin: { frm: './bin/framerize-server.js', framerize: './bin/framerize-server.js', 'framerize-server': './bin/framerize-server.js' },
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

test('private packages and extra package entries are rejected', (t) => {
  for (const [change, error] of [
    [{ private: true }, 'Server package must permit npm publication'],
    [{ files: ['bin', ...generated.map((name) => `dist/${name}`),
      ...notices, 'dist/extra'] }, 'Unexpected server package files'],
    [{ exports: { '.': './src/server.js' } }, 'Unexpected server package exports'],
    [{ bin: { frm: './bin/framerize-server.js', framerize: './bin/framerize-server.js', 'framerize-server': './bin/framerize-server.js', extra: './bin/extra.js' } }, 'Unexpected server package executable'],
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
    ['src', 'extra.ts', 'Unexpected server source files'],
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
