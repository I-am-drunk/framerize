import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageRoot = resolve(process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const manifest = JSON.parse(await readFile(join(packageRoot, 'package.json'), 'utf8'));

for (const flag of ['version', '--version', '-v']) {
  test(`Server launcher ${flag} reports its package version without setup`, async context => {
    assert.equal(manifest.bin.frm, manifest.bin.framerize);
    const root = await mkdtemp(join(tmpdir(), 'server-cli-version-'));
    context.after(() => rm(root, { recursive: true, force: true }));
    const canary = 'synthetic settings must stay unchanged\n';
    await writeFile(join(root, 'server.json'), canary);
    const result = spawnSync(process.execPath, [resolve(packageRoot, manifest.bin.frm), flag], {
      cwd: root, encoding: 'utf8', timeout: 5000, maxBuffer: 64 * 1024,
      env: { HOME: root, USERPROFILE: root, TMPDIR: root, TEMP: root, TMP: root,
        FRAMERIZE_SERVER_HOME: root, FRAMERIZE_SERVER_PORT: 'invalid',
        NODE_DISABLE_COMPILE_CACHE: '1',
        ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) },
    });
    assert.equal(result.error, undefined);
    assert.equal(result.signal, null);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stderr, '');
    assert.equal(result.stdout, `${manifest.version}\n`);
    assert.equal(await readFile(join(root, 'server.json'), 'utf8'), canary);
    assert.deepEqual(await readdir(root), ['server.json']);
  });
}
