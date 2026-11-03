import { createHash } from 'node:crypto';
import { copyFileSync, lstatSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import assert from 'node:assert/strict';

export interface RuntimeFile { readonly path: string; readonly bytes: number; readonly sha256: string; }

export function bundleCastleRuntime(archive: ReadonlyMap<string, Buffer>): Buffer | undefined {
  const source = archive.get('package/dist/castle-manifest.json');
  if (!source) {
    assert.ok(!archive.has('package/dist/castle-loader.js'), 'Castle manifest missing');
    return undefined;
  }
  const manifest = JSON.parse(source.toString('utf8')) as { version: number; files: RuntimeFile[] };
  assert.equal(manifest.version, 1);
  assert.ok(Array.isArray(manifest.files) && manifest.files.length >= 2 && manifest.files.length <= 256);
  const paths = manifest.files.map(file => `package/dist/castle/${file.path}`);
  assert.deepEqual([...archive.keys()].filter(path => path.startsWith('package/dist/castle/')).sort(),
    [...new Set(paths)].sort(), 'Castle runtime inventory differs');
  assert.equal(new Set(paths).size, paths.length);
  return runtimeBytes(archive, manifest.files);
}

function runtimeBytes(archive: ReadonlyMap<string, Buffer>, manifest: RuntimeFile[]): Buffer {
  let total = 0;
  const files = manifest.map(file => {
    assert.ok(typeof file.path === 'string' && file.path.split('/').every(part =>
      /^[A-Za-z0-9_.-]+$/.test(part) && part !== '.' && part !== '..'));
    const bytes = archive.get(`package/dist/castle/${file.path}`)!;
    assert.equal(bytes.length, file.bytes, 'Castle runtime size differs');
    assert.equal(createHash('sha256').update(bytes).digest('hex'), file.sha256, 'Castle runtime hash differs');
    total += bytes.length;
    return { ...file, data: bytes.toString('base64') };
  });
  assert.ok(total <= 8 * 1024 * 1024);
  assert.ok(files.some(file => file.path === 'package.json') && files.some(file => file.path === 'dist/castle.js'));
  return Buffer.from(JSON.stringify({ version: 1, files }));
}

export function stageCastleRuntime(root: string): void {
  const source = join(root, 'framerize-castle');
  const target = join(root, 'framerize-server/dist/castle');
  const manifest = JSON.parse(readFileSync(join(source, 'package.json'), 'utf8')) as { files: string[] };
  const paths = ['package.json', ...manifest.files].flatMap(path => inventory(source, path)).sort();
  if (paths.length > 256 || new Set(paths).size !== paths.length) throw new Error('Invalid Castle inventory');
  const files = paths.map(path => {
    const bytes = readFileSync(join(source, path));
    return { path, bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') };
  });
  if (files.reduce((total, file) => total + file.bytes, 0) > 8 * 1024 * 1024) throw new Error('Castle runtime is too large');
  writeRuntime(source, target, files);
}

function inventory(root: string, path: string): string[] {
  if (!path.split('/').every(part => /^[A-Za-z0-9_.-]+$/.test(part) && part !== '.' && part !== '..')) {
    throw new Error('Invalid Castle path');
  }
  const file = join(root, path);
  const info = lstatSync(file);
  if (info.isFile()) return [path];
  if (!info.isDirectory()) throw new Error('Castle runtime contains a special file');
  return readdirSync(file).flatMap(name => inventory(root, `${path}/${name}`));
}

function writeRuntime(source: string, target: string, files: RuntimeFile[]): void {
  rmSync(target, { recursive: true, force: true });
  for (const file of files) {
    const destination = join(target, file.path);
    mkdirSync(dirname(destination), { recursive: true });
    copyFileSync(join(source, file.path), destination);
  }
  writeFileSync(join(dirname(target), 'castle-manifest.json'), JSON.stringify({ version: 1, files }) + '\n');
}
