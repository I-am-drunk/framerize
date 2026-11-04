import assert from 'node:assert/strict';
import { readFile, readdir, lstat } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const manifest = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));
assert.equal(manifest.name, 'framerize-server', 'Unexpected server package name');
assert.equal(manifest.private, true, 'Server package must remain private');
const generated = ['accounts.d.ts', 'accounts.js', 'config.d.ts', 'config.js', 'core.js', 'core.d.ts',
  'backend.d.ts', 'backend.js', 'castle-loader.d.ts', 'castle-loader.js',
  'castle-assets.d.ts', 'castle-assets.js', 'castle-manifest.json',
  'account-backend.d.ts', 'account-backend.js',
  ...['backend', 'http-policy', 'instance', 'instance-options', 'readiness'].map((name) => `core-types/${name}.d.ts`),
  'errors.d.ts', 'errors.js', 'provider.d.ts', 'provider.js',
  'framer-auth.d.ts', 'framer-auth.js',
  'framer-backend.d.ts', 'framer-backend.js', 'framer-request.d.ts', 'framer-request.js',
  ...['completed-reply', 'completed-stream', 'events', 'messages-stream', 'responses-stream']
    .flatMap(name => [`compat/${name}.js`, `compat/${name}.d.ts`]),
  'cli.d.ts', 'cli.js', 'logger.d.ts', 'logger.js', 'server.d.ts', 'server.js'];
assert.deepEqual([...(manifest.files ?? [])].sort(),
  ['LICENSE', 'README.md', 'bin', 'dist/castle', ...generated.map((name) => `dist/${name}`)].sort(),
  'Unexpected server package files');
assert.deepEqual(manifest.exports, {
  '.': './dist/server.js', './config': './dist/config.js', './accounts': './dist/accounts.js',
  './logger': './dist/logger.js', './provider': './dist/provider.js',
  './framer-auth': './dist/framer-auth.js',
  './framer-backend': './dist/framer-backend.js',
  './account-backend': './dist/account-backend.js',
}, 'Unexpected server package exports');
assert.deepEqual(manifest.bin, { 'framerize-server': './bin/framerize-server.js' },
  'Unexpected server package executable');
for (const field of ['dependencies', 'optionalDependencies', 'peerDependencies']) {
  assert.deepEqual(Object.keys(manifest[field] ?? {}), [], 'Unexpected runtime dependencies');
}
assert.deepEqual((await readdir(new URL('../src/', import.meta.url))).sort(), [
  'account-backend.ts', 'accounts.ts', 'backend.ts', 'castle-assets.ts', 'castle-loader.ts',
  'cli.ts', 'compat', 'config.ts', 'errors.ts', 'framer-auth.ts', 'framer-backend.ts',
  'framer-request.ts', 'logger.ts', 'provider.ts', 'server.ts',
], 'Unexpected server source files');
assert.deepEqual((await readdir(new URL('../src/compat/', import.meta.url))).sort(),
  ['completed-reply.ts', 'completed-stream.ts', 'events.ts', 'messages-stream.ts', 'responses-stream.ts'],
  'Unexpected compatibility source files');
assert.deepEqual(await readdir(new URL('../bin/', import.meta.url)), ['framerize-server.js'],
  'Unexpected server executable files');
for (const name of generated) assert.ok((await lstat(new URL(`../dist/${name}`, import.meta.url))).isFile(),
  'Missing compiled server module');

for (const directory of ['src', 'bin', 'scripts', 'dist', 'dist/compat']) {
  for (const name of await readdir(new URL(`../${directory}/`, import.meta.url))) {
    if (!name.endsWith('.js')) continue;
    const file = new URL(`../${directory}/${name}`, import.meta.url);
    const result = spawnSync(process.execPath, ['--check', fileURLToPath(file)], { stdio: 'inherit' });
    if (result.error || result.status !== 0) process.exit(1);
  }
}
