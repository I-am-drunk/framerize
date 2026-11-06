import assert from 'node:assert/strict';
import test, { type TestContext } from 'node:test';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { gzipSync } from 'node:zlib';
import { readPackageArchive } from '../scripts/package-archive.js';

type Entry = { name: string; body?: string; kind?: string; signature?: string; prefix?: string };
for (const kind of ['1', '2', '3', '4', '6', 'x', 'g', 'L']) {
  test(`rejects link, special or extended archive entry ${kind}`, context => {
    assert.throws(read(context, tar([manifest, { name: 'package/member', kind }])));
  });
}

test('rejects duplicate and case-colliding members', context => {
  assert.throws(read(context, tar([manifest, manifest])), /Duplicate/);
  assert.throws(read(context, tar([manifest, { name: 'package/PACKAGE.json' }])), /Duplicate/);
});

test('rejects corrupt headers, missing end blocks and trailing data', context => {
  const valid = tar([manifest]);
  const corrupt = Buffer.from(valid); corrupt[4] = 65;
  assert.throws(read(context, corrupt), /checksum/);
  assert.throws(read(context, valid.subarray(0, -1024)), /Incomplete/);
  assert.throws(read(context, valid.subarray(0, -1)), /Truncated/);
  assert.throws(read(context, Buffer.concat([valid, Buffer.alloc(512, 1)])), /archive end/);
});
const manifest = { name: 'package/package.json', body: '{"name":"fixture"}' };
for (const [name, signature] of [
  ['GNU', 'ustar  \0'], ['versionless', 'ustar\0\0\0'], ['unknown version', 'ustar\0' + '01'],
] as const) {
  test(`rejects ${name} signatures before using the POSIX prefix`, context => {
    const bytes = tar([{ name: 'package.json', body: manifest.body, signature, prefix: 'package' }]);
    assert.throws(read(context, bytes), /Unsupported archive format/);
  });
}

test('reads the POSIX prefix with its exact magic and version', context => {
  const result = read(context, tar([{ name: 'package.json', body: manifest.body, prefix: 'package' }]))();
  assert.deepEqual([...result], [[manifest.name, Buffer.from(manifest.body)]]);
});

test('rejects file/directory conflicts and excessive decompression', context => {
  assert.throws(read(context, tar([manifest, { name: 'package/path' }, { name: 'package/path/child' }])), /directory/);
  assert.throws(read(context, tar([manifest, { name: 'package/path/child' }, { name: 'package/path' }])), /directory/);
  assert.throws(read(context, Buffer.alloc(32 * 1024 * 1024 + 512)));
});
function read(context: TestContext, bytes: Buffer) {
  const directory = mkdtempSync(join(tmpdir(), 'package-archive-'));
  context.after(() => rmSync(directory, { recursive: true, force: true }));
  const file = join(directory, 'package.tgz');
  writeFileSync(file, gzipSync(bytes));
  return () => readPackageArchive(file);
}

test('reads regular package bytes and ignores safe directory entries', context => {
  const result = read(context, tar([{ name: 'package/', kind: '5' }, manifest]))();
  assert.deepEqual([...result], [[manifest.name, Buffer.from(manifest.body)]]);
});

for (const name of ['/outside', 'package/../outside', 'package/./file', 'package//file',
  'package/file.', 'package/CON.txt', 'package/dir\\file', 'other/file']) {
  test(`rejects unsafe archive path ${name}`, context => {
    assert.throws(read(context, tar([manifest, { name }])));
  });
}
function tar(entries: Entry[]): Buffer {
  const chunks: Buffer[] = [];
  for (const { name, body = '', kind = '0', signature = 'ustar\0' + '00', prefix = '' } of entries) {
    const data = Buffer.from(body);
    const header = Buffer.alloc(512);
    header.write(name); header.write(data.length.toString(8).padStart(11, '0'), 124);
    header.fill(32, 148, 156); header.write(kind, 156); header.write(signature, 257);
    header.write(prefix, 345);
    header.write(header.reduce((total, byte) => total + byte, 0).toString(8).padStart(6, '0') + '\0 ', 148);
    chunks.push(header, data, Buffer.alloc((512 - data.length % 512) % 512));
  }
  return Buffer.concat([...chunks, Buffer.alloc(1024)]);
}
