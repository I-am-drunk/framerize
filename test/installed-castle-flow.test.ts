import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import { copyFileSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { test } from 'node:test';
import { npmCli, privateEnvironment } from '../scripts/core-package-check.js';
import { assetManifest } from '../framerize-castle/src/asset-manifest.js';

const repository = fileURLToPath(new URL('../../', import.meta.url));
const modules = ['asset-manifest', 'asset-snapshot', 'castle', 'collect', 'collect-worker',
  'collection-guard', 'encode', 'environment', 'errors', 'lifecycle', 'options',
  'snapshot-loader', 'trace', 'types', 'worker', 'encoder/serialize',
  'encoder/compress', 'encoder/compression-huffman', 'encoder/compression-matches'];
const expected = ['package.json', 'README.md', 'LICENSE',
  ...modules.flatMap((name) => [`dist/${name}.js`, `dist/${name}.d.ts`]),
  ...assetManifest.map((entry) => entry.path)].sort();

test('private Castle package works outside the workspace without runtime dependencies', { timeout: 60000 }, (context) => {
  const directory = realpathSync(mkdtempSync(join(tmpdir(), 'castle-installed-')));
  const source = join(repository, 'framerize-castle');
  try {
    const consumer = join(directory, 'consumer with spaces');
    mkdirSync(consumer);
    writeFileSync(join(consumer, 'package.json'), '{"type":"module","private":true}');
    writeFileSync(join(directory, 'user.npmrc'), '');
    writeFileSync(join(directory, 'global.npmrc'), '');
    const env = privateEnvironment(directory);
    const run = (args: string[], cwd = consumer, environment = env) => execFileSync(process.execPath, args,
      { cwd, env: environment, encoding: 'utf8', timeout: 25000, killSignal: 'SIGKILL', maxBuffer: 2 * 1024 * 1024 });
    const packed = JSON.parse(run([npmCli(), 'pack', source, '--json', '--ignore-scripts',
      '--pack-destination', directory])) as Array<{ filename: string; size: number; files: Array<{ path: string }> }>;
    assert.equal(packed.length, 1);
    const info = packed[0]!;
    assert.match(info.filename, /^framerize-castle-0\.1\.0\.tgz$/);
    assert.deepEqual(info.files.map((file) => file.path).sort(), expected);
    assert.ok(info.size > 0 && info.size < 2 * 1024 * 1024);
    const archive = join(directory, info.filename);
    const digest = createHash('sha256').update(readFileSync(archive)).digest('hex');
    run([npmCli(), 'install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund', archive]);
    const installed = join(consumer, 'node_modules', 'framerize-castle');
    for (const path of expected) assert.deepEqual(readFileSync(join(installed, path)), readFileSync(join(source, path)));
    assert.deepEqual(inventory(installed), expected);
    const manifest = JSON.parse(readFileSync(join(installed, 'package.json'), 'utf8'));
    assert.equal(manifest.private, true);
    for (const field of ['dependencies', 'optionalDependencies', 'peerDependencies']) {
      assert.deepEqual(manifest[field] ?? {}, {});
    }
    const entry = createRequire(join(consumer, 'package.json')).resolve('framerize-castle');
    assert.equal(entry, join(installed, 'dist', 'castle.js'));
    writeFileSync(join(consumer, 'consumer.ts'), [
      "import { createCastleMinter, type CastleMinter, type MintResult } from 'framerize-castle';",
      "const minter: CastleMinter = createCastleMinter({ encoderPath: '/synthetic/encoder' });",
      'const result: Promise<MintResult> = minter.mint({ signal: new AbortController().signal });',
      'void result; void minter.close();',
    ].join('\n'));
    const compiler = join(repository, 'node_modules/typescript/bin/tsc');
    const flags = ['--strict', '--noEmit', '--target', 'ES2022', '--module', 'NodeNext',
      '--moduleResolution', 'NodeNext', '--typeRoots', join(repository, 'node_modules/@types')];
    run([compiler, 'consumer.ts', ...flags]);
    writeFileSync(join(consumer, 'invalid.ts'), [
      "import { createCastleMinter } from 'framerize-castle';",
      "const invalid: number = createCastleMinter({ encoderPath: '/synthetic/encoder' });",
    ].join('\n'));
    assert.throws(() => run([compiler, 'invalid.ts', ...flags]), (error: unknown) =>
      error !== null && typeof error === 'object' && 'stdout' in error && /TS2322/.test(String(error.stdout)));
    const output = run(['--test', join(source, '.test-dist/checks/integration.js')], consumer,
      { ...env, CASTLE_ENTRY: entry });
    assert.match(output, /(?:#|ℹ) pass 3/);
    context.diagnostic(output.trim());
    const destination = process.env.CASTLE_PACKAGE_OUTPUT;
    if (destination) {
      mkdirSync(destination, { recursive: true });
      copyFileSync(archive, join(destination, info.filename));
      writeFileSync(join(destination, 'pack.json'), JSON.stringify(info, null, 2));
      writeFileSync(join(destination, 'integration.log'), output);
    }
    context.diagnostic(JSON.stringify({ node: process.version, sha256: digest, bytes: info.size,
      files: expected.length, installedIntegration: 3 }));
  } finally { rmSync(directory, { recursive: true, force: true }); }
});

function inventory(root: string, prefix = ''): string[] {
  const files: string[] = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const relative = prefix ? `${prefix}/${entry.name}` : entry.name;
    if (entry.isDirectory()) files.push(...inventory(join(root, entry.name), relative));
    else {
      assert.ok(entry.isFile(), 'Package contains a special file');
      files.push(relative);
    }
  }
  return files.sort();
}
