import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { build } from 'esbuild';

const entry = fileURLToPath(new URL('../src/present.ts', import.meta.url));
const manifestPath = fileURLToPath(new URL('../package.json', import.meta.url));
const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));
const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const installed = JSON.parse(await readFile(join(packageRoot, 'package.json'), 'utf8'));

for (const flag of ['version', '--version', '-v']) {
  test(`Remote launcher ${flag} reports its package version without setup`, async context => {
    assert.equal(installed.bin.frm, installed.bin.framerize);
    const root = await mkdtemp(join(tmpdir(), 'remote-cli-version-'));
    context.after(() => rm(root, { recursive: true, force: true }));
    const result = spawnSync(process.execPath, [resolve(packageRoot, installed.bin.frm), flag], {
      cwd: root, encoding: 'utf8', timeout: 5000, maxBuffer: 64 * 1024,
      env: { HOME: root, USERPROFILE: root, TMPDIR: root, TEMP: root, TMP: root,
        FRM_HOME: root, FRM_PORT: 'invalid', FRM_REMOTE: 'not a URL',
        NODE_DISABLE_COMPILE_CACHE: '1',
        ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) },
    });
    assert.equal(result.error, undefined);
    assert.equal(result.signal, null);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.stderr, '');
    assert.equal(result.stdout, `${installed.version}\n`);
    assert.deepEqual(await readdir(root), []);
  });
}

for (const version of ['3.4.5', '3.4.6-alpha.17+abc1234']) {
  test(`Remote bundle reports manifest version ${version}`, async () => {
    const result = await build({ entryPoints: [entry], bundle: true, write: false,
      platform: 'node', format: 'esm', target: 'node22.12', logLevel: 'silent',
      plugins: [{ name: 'test-manifest-version', setup(builder) {
        builder.onLoad({ filter: /package\.json$/ }, args => args.path === manifestPath
          ? { contents: JSON.stringify({ ...manifest, version }), loader: 'json' } : undefined);
      } }],
    });
    const output = Buffer.from(result.outputFiles[0].contents).toString('base64');
    const exported = await import(`data:text/javascript;base64,${output}#${version}`);
    assert.equal(exported.VERSION, version);
    assert.ok(exported.HELP.startsWith(`frm ${version}\n`));
    assert.ok(exported.FRM_HELP.startsWith(`frm ${version}\n`));
  });
}
