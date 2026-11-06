import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const root = resolve(process.env.FRAMERIZE_REMOTE_SOURCE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const compiled = await build({ entryPoints: [join(root, 'src/compatibility/versions.ts')],
  bundle: true, platform: 'node', format: 'esm', write: false });
const { readText } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const limit = 1024 * 1024;

async function fixture(context) {
  const directory = await fs.mkdtemp(join(tmpdir(), 'frm-settings-read-limit-'));
  context.after(() => fs.rm(directory, { recursive: true, force: true }));
  return join(directory, 'settings.json');
}

test('oversized regular settings are rejected without reading the entire file', async context => {
  const file = await fixture(context);
  const handle = await fs.open(file, 'wx', 0o600);
  try { await handle.truncate(8 * limit); } finally { await handle.close(); }
  let bytesRead = 0;
  const opened = [];
  const originalReadFile = fs.readFile, originalOpen = fs.open;
  const count = value => { bytesRead += Buffer.isBuffer(value) ? value.length : Buffer.byteLength(value); };
  context.after(async () => {
    try { await Promise.all(opened.filter(state => !state.closed).map(state => state.close())); }
    finally { context.mock.restoreAll(); syncBuiltinESMExports(); }
  });
  context.mock.method(fs, 'readFile', async (path, ...options) => {
    const value = await originalReadFile(path, ...options);
    if (path === file) count(value);
    return value;
  });
  context.mock.method(fs, 'open', async (path, ...options) => {
    const current = await originalOpen(path, ...options);
    if (path !== file) return current;
    const state = { closed: false, close: () => current.close() };
    opened.push(state);
    const read = current.read.bind(current), readFile = current.readFile.bind(current);
    const close = current.close.bind(current);
    current.read = async (...options) => {
      const result = await read(...options);
      bytesRead += result.bytesRead;
      return result;
    };
    current.readFile = async (...options) => {
      const value = await readFile(...options);
      count(value);
      return value;
    };
    current.close = async () => { await close(); state.closed = true; };
    return current;
  });
  syncBuiltinESMExports();
  assert.equal(await readText(file), null);
  assert.ok(opened.every(state => state.closed), 'Every captured file handle must close');
  context.diagnostic(JSON.stringify({ fixtureBytes: 8 * limit, bytesRead, opened }));
  assert.ok(bytesRead <= 3 * limit + 65536,
    `Read ${bytesRead} bytes despite the ${limit}-code-unit text limit`);
});

test('bounded settings reads preserve UTF-8 decoding and the code-unit limit', async context => {
  const file = await fixture(context);
  const samples = [
    Buffer.concat([Buffer.alloc(65535, 97), Buffer.from('é終🙂')]),
    Buffer.from([0x61, 0xe2, 0x82]),
    Buffer.from('終'.repeat(limit)),
    Buffer.from('🙂'.repeat(limit / 2)),
    Buffer.from('終'.repeat(limit + 1)),
  ];
  for (const bytes of samples) {
    const expected = bytes.toString('utf8');
    await fs.writeFile(file, bytes);
    assert.equal(await readText(file), expected.length <= limit ? expected : null);
  }
});
