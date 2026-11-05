import assert from 'node:assert/strict';
import test from 'node:test';
import { join } from 'node:path';
import { appendFileSync, copyFileSync, cpSync, mkdtempSync, readdirSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { checkCorePackage, packagePolicy, privateEnvironment } from '../scripts/core-package-check.js';

const paths = ['README.md', 'package.json', 'dist/index.js', 'dist/index.d.ts'];
const valid = { filename: 'framerize-core-0.1.0.tgz', size: 8000, unpackedSize: 28000,
  files: paths.map((path) => ({ path, size: 100 })) };

const staleCases = [
  { name: 'modified declarations', file: 'dist/index.d.ts',
    content: '\nexport declare const staleArtifact: true;\n', error: /Core archive differs from a clean build/ },
  { name: 'orphaned JavaScript', file: 'dist/removed-module.js',
    content: 'export {};\n', error: /Unexpected package inventory/ },
];

for (const variant of staleCases) {
  test(`clean rebuild rejects ${variant.name}`, { timeout: 30_000 }, () => {
    const root = mkdtempSync(join(tmpdir(), 'core-stale-build-'));
    try {
      const source = join(root, 'framerize-core');
      cpSync(new URL('../../framerize-core/dist/', import.meta.url), join(source, 'dist'), { recursive: true });
      cpSync(new URL('../../framerize-core/src/', import.meta.url), join(source, 'src'), { recursive: true });
      for (const name of ['README.md', 'package.json', 'tsconfig.build.json']) {
        copyFileSync(new URL(`../../framerize-core/${name}`, import.meta.url), join(source, name));
      }
      copyFileSync(new URL('../../tsconfig.base.json', import.meta.url), join(root, 'tsconfig.base.json'));
      appendFileSync(join(source, variant.file), variant.content);
      assert.throws(() => checkCorePackage(root), variant.error);
    } finally { rmSync(root, { recursive: true, force: true }); }
  });
}

test('failed preflight removes its private temporary directory', () => {
  const root = mkdtempSync(join(tmpdir(), 'core-check-cleanup-'));
  const names = ['TMPDIR', 'TMP', 'TEMP'];
  const original = Object.fromEntries(names.map((name) => [name, process.env[name]]));
  try {
    for (const name of names) process.env[name] = root;
    assert.throws(() => checkCorePackage(root), { code: 'ENOENT' });
    assert.deepEqual(readdirSync(root), []);
  } finally {
    for (const name of names) {
      if (original[name] === undefined) delete process.env[name];
      else process.env[name] = original[name];
    }
    rmSync(root, { recursive: true, force: true });
  }
});

test('accepts exactly the expected package inventory', () => {
  assert.doesNotThrow(() => packagePolicy(valid, paths));
  assert.doesNotThrow(() => packagePolicy({ ...valid, files: [...valid.files].reverse() }, paths));
});

test('CI covers both pinned runtimes on all platforms without native builds', () => {
  const workflow = readFileSync(new URL('../../.github/workflows/core-ci.yml', import.meta.url), 'utf8');
  assert.match(workflow, /os: \[ubuntu-22\.04, macos-15, windows-2022\]/);
  assert.match(workflow, /node: \['22\.23\.4', '24\.13\.0'\]/);
  assert.match(workflow, /persist-credentials: false/);
  assert.match(workflow, /npm ci --ignore-scripts --no-audit --no-fund/);
  assert.match(workflow, /npm run build && npm run check/);
  assert.match(workflow, /run: npm test/);
  assert.match(workflow, /core-package-check\.js > "\$RUNNER_TEMP\//);
  assert.match(workflow, /if-no-files-found: error/);
  assert.match(workflow, /\n  workflow_call:/);
  assert.doesNotMatch(workflow, /\n  (push|pull_request):/);
  assert.doesNotMatch(workflow,
    /build:native|npm publish|secrets\./);
});

test('rejects missing, unexpected and duplicate package members', () => {
  for (const files of [valid.files.slice(1), [...valid.files, { path: 'node_modules/x.js', size: 1 }],
    [...valid.files, valid.files[0]!]]) {
    assert.throws(() => packagePolicy({ ...valid, files }, paths), /Unexpected package inventory/);
  }
});

test('rejects archive traversal and size-budget violations', () => {
  for (const filename of ['../framerize-core-0.1.0.tgz', 'framerize-core-..\\x.tgz', '/tmp/file.tgz']) {
    assert.throws(() => packagePolicy({ ...valid, filename }, paths));
  }
  for (const size of [0, -1, 1.5, Infinity, NaN, 65537]) {
    assert.throws(() => packagePolicy({ ...valid, size }, paths), /64 KiB/);
  }
  assert.throws(() => packagePolicy({ ...valid, unpackedSize: 262145 }, paths), /256 KiB/);
});

test('private npm environment drops inherited credentials, proxies and startup hooks', () => {
  const names = ['OPENAI_API_KEY', 'NODE_OPTIONS', 'NODE_PATH', 'HTTPS_PROXY',
    'NODE_USE_ENV_PROXY', 'npm_config_registry', 'npm_config_userconfig'];
  const original = Object.fromEntries(names.map((name) => [name, process.env[name]]));
  try {
    for (const name of names) process.env[name] = 'synthetic-private-value';
    const root = join(process.cwd(), 'synthetic-private-root');
    const env = privateEnvironment(root);
    for (const name of names) assert.notEqual(env[name], 'synthetic-private-value');
    assert.equal(env.HOME, root);
    assert.equal(env.USERPROFILE, root);
    assert.equal(env.npm_config_userconfig, join(root, 'user.npmrc'));
    assert.equal(env.npm_config_globalconfig, join(root, 'global.npmrc'));
    assert.equal(env.npm_config_offline, 'true');
    assert.equal(env.npm_config_ignore_scripts, 'true');
    assert.equal(env.NODE_OPTIONS, '');
    assert.equal(env.NODE_DISABLE_COMPILE_CACHE, '1');
  } finally {
    for (const name of names) {
      if (original[name] === undefined) delete process.env[name];
      else process.env[name] = original[name];
    }
  }
});
