import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile, rm, stat } from 'node:fs/promises';
import { join } from 'node:path';
import test from 'node:test';
import { decodeRuntime, materializeRuntime } from '../dist/castle-assets.js';
import { createServerMinter } from '../dist/castle-loader.js';

const asset = (path, data) => ({ path, bytes: Buffer.byteLength(data),
  sha256: createHash('sha256').update(data).digest('hex'), data: Buffer.from(data).toString('base64') });
const files = () => [asset('package.json', '{"type":"module"}'), asset('dist/castle.js', 'export {};')];
const encoded = entries => new TextEncoder().encode(JSON.stringify({ version: 1, files: entries })).buffer;

test('embedded runtime rejects traversal, substitutions, duplicate paths and missing entries', () => {
  assert.equal(decodeRuntime(encoded(files())).length, 2);
  for (const entries of [[...files(), asset('../escape', 'x')], [...files(), files()[0]],
    [files()[0]], [files()[0], { ...files()[1], sha256: '0'.repeat(64) }],
    [files()[0], { ...files()[1], bytes: 0 }]]) assert.throws(() => decodeRuntime(encoded(entries)));
});

test('embedded files are materialized privately without source changes', async () => {
  const directory = await materializeRuntime(encoded(files()), new AbortController().signal);
  try {
    assert.equal(await readFile(join(directory, 'dist/castle.js'), 'utf8'), 'export {};');
    if (process.platform !== 'win32') {
      assert.equal((await stat(directory)).mode & 0o777, 0o700);
      assert.equal((await stat(join(directory, 'dist/castle.js'))).mode & 0o777, 0o600);
    }
  } finally { await rm(directory, { recursive: true, force: true }); }
});

test('embedded runtime enforces byte limits and canonical encoding before writing', () => {
  assert.throws(() => decodeRuntime(new ArrayBuffer(16 * 1024 * 1024 + 1)));
  for (const replacement of [{ data: `${files()[1].data}\n` }, { bytes: -1 },
    { path: '/dist/castle.js' }, { path: 'dist/../castle.js' }]) {
    assert.throws(() => decodeRuntime(encoded([files()[0], { ...files()[1], ...replacement }])));
  }
});

test('minter startup and materialization reject cancellation before work', async () => {
  const signal = AbortSignal.abort(new Error('synthetic-private'));
  await assert.rejects(materializeRuntime(encoded(files()), signal));
  await assert.rejects(createServerMinter(signal), { message: 'Castle runtime unavailable' });
});
