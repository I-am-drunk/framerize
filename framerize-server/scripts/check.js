import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const manifest = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));
assert.equal(manifest.name, 'framerize-server', 'Unexpected server package name');
assert.equal(manifest.private, true, 'Server package must remain private');
assert.deepEqual([...(manifest.files ?? [])].sort(), ['LICENSE', 'README.md', 'bin', 'src'],
  'Unexpected server package files');
assert.deepEqual(manifest.exports, {
  '.': './src/server.js', './config': './src/config.js', './accounts': './src/accounts.js',
  './logger': './src/logger.js', './provider': './src/provider.js',
}, 'Unexpected server package exports');
assert.deepEqual(manifest.bin, { 'framerize-server': './bin/framerize-server.js' },
  'Unexpected server package executable');
assert.deepEqual(Object.keys(manifest.dependencies ?? {}), [], 'Unexpected runtime dependencies');
assert.deepEqual((await readdir(new URL('../src/', import.meta.url))).sort(), [
  'accounts.js', 'cli.js', 'config.js', 'errors.js', 'lifecycle.js', 'logger.js',
  'provider.js', 'server.js', 'transport.js',
], 'Unexpected server source files');
assert.deepEqual(await readdir(new URL('../bin/', import.meta.url)), ['framerize-server.js'],
  'Unexpected server executable files');

for (const directory of ['src', 'bin', 'scripts']) {
  for (const name of await readdir(new URL(`../${directory}/`, import.meta.url))) {
    if (!name.endsWith('.js')) continue;
    const file = new URL(`../${directory}/${name}`, import.meta.url);
    const result = spawnSync(process.execPath, ['--check', fileURLToPath(file)], { stdio: 'inherit' });
    if (result.error || result.status !== 0) process.exit(1);
  }
}
