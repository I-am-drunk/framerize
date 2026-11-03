import { createHash } from 'node:crypto';
import { mkdir, mkdtemp, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';

interface Asset { readonly path: string; readonly bytes: Buffer; }

export function decodeRuntime(input: ArrayBuffer): readonly Asset[] {
  if (input.byteLength > 16 * 1024 * 1024) throw new Error();
  const manifest = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(input)) as {
    version?: unknown; files?: unknown;
  };
  if (manifest?.version !== 1 || !Array.isArray(manifest.files)
    || manifest.files.length < 2 || manifest.files.length > 256) throw new Error();
  const paths = new Set<string>();
  let total = 0;
  const files = manifest.files.map((entry: unknown) => {
    const file = decodeAsset(entry);
    total += file.bytes.length;
    if (paths.has(file.path) || total > 8 * 1024 * 1024) throw new Error();
    paths.add(file.path);
    return file;
  });
  if (!paths.has('package.json') || !paths.has('dist/castle.js')) throw new Error();
  return files;
}

function decodeAsset(input: unknown): Asset {
  if (!input || typeof input !== 'object') throw new Error();
  const file = input as Record<string, unknown>;
  if (typeof file.path !== 'string' || file.path.length > 240
    || !file.path.split('/').every(part => /^[A-Za-z0-9_.-]+$/.test(part) && part !== '.' && part !== '..')
    || !/^(?:dist\/|castle\/|package\.json$|README\.md$|LICENSE$)/.test(file.path)) throw new Error();
  if (typeof file.data !== 'string' || file.data.length > 12 * 1024 * 1024
    || typeof file.sha256 !== 'string' || !/^[a-f0-9]{64}$/.test(file.sha256)
    || !Number.isSafeInteger(file.bytes)) throw new Error();
  const bytes = Buffer.from(file.data, 'base64');
  if (bytes.length !== file.bytes || bytes.toString('base64') !== file.data
    || createHash('sha256').update(bytes).digest('hex') !== file.sha256) throw new Error();
  return { path: file.path, bytes };
}

export async function materializeRuntime(input: ArrayBuffer, signal: AbortSignal): Promise<string> {
  signal.throwIfAborted();
  const files = decodeRuntime(input);
  const created = await mkdtemp(join(tmpdir(), 'framerize-castle-runtime-'));
  try {
    const root = await realpath(created);
    for (const file of files) {
      signal.throwIfAborted();
      const destination = join(root, file.path);
      await mkdir(dirname(destination), { recursive: true, mode: 0o700 });
      await writeFile(destination, file.bytes, { flag: 'wx', mode: 0o600 });
    }
    signal.throwIfAborted();
    return root;
  } catch (error) {
    await rm(created, { recursive: true, force: true });
    throw error;
  }
}
