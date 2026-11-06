import assert from 'node:assert/strict';
import { readFileSync, statSync } from 'node:fs';
import { gunzipSync } from 'node:zlib';

export function readPackageArchive(input: string | Buffer): Map<string, Buffer> {
  if (typeof input === 'string') {
    const info = statSync(input);
    assert.ok(info.isFile() && info.size <= 16 * 1024 * 1024, 'Invalid archive size or type');
  }
  const compressed = typeof input === 'string' ? readFileSync(input) : input;
  assert.ok(compressed.length <= 16 * 1024 * 1024, 'Invalid archive size');
  const bytes = gunzipSync(compressed, { maxOutputLength: 32 * 1024 * 1024 });
  assert.equal(bytes.length % 512, 0, 'Truncated archive block');
  const entries = new Map<string, Buffer>();
  const seen = new Set<string>();
  const files = new Set<string>();
  let position = 0;
  let ended = false;
  while (position + 512 <= bytes.length) {
    const header = bytes.subarray(position, position + 512);
    position += 512;
    if (header.every(byte => byte === 0)) {
      assert.ok(bytes.length - position >= 512 && bytes.subarray(position).every(byte => byte === 0), 'Invalid archive end');
      ended = true; break;
    }
    const field = (start: number, size: number) => header.subarray(start, start + size).toString('utf8').split('\0')[0]!;
    const octal = (start: number, size: number) => {
      const value = field(start, size).trim();
      assert.match(value, /^[0-7]+$/, 'Unsupported archive number');
      return Number.parseInt(value, 8);
    };
    const checksum = header.reduce((sum, byte, index) => sum + (index >= 148 && index < 156 ? 32 : byte), 0);
    assert.equal(octal(148, 8), checksum, 'Archive checksum mismatch');
    // Byte 345 is a pathname prefix only in the POSIX USTAR layout.
    assert.equal(header.subarray(257, 265).toString('latin1'), 'ustar\0' + '00', 'Unsupported archive format');
    const prefix = field(345, 155);
    const name = (prefix ? `${prefix}/` : '') + field(0, 100);
    const kind = header[156];
    assert.ok(kind === 0 || kind === 48 || kind === 53, 'Only regular archive files and directories are allowed');
    const size = octal(124, 12);
    assert.ok(Number.isSafeInteger(size) && position + size <= bytes.length, 'Truncated archive entry');
    const directory = kind === 53;
    const canonical = directory ? name.replace(/\/$/, '') : name;
    assert.ok(canonical === 'package' ? directory : canonical.startsWith('package/'), 'Unsafe archive root');
    assert.match(canonical, /^[a-zA-Z0-9_./-]+$/, 'Unsupported archive member');
    for (const segment of canonical.split('/')) {
      assert.ok(segment && segment !== '.' && segment !== '..' && !segment.endsWith('.'), 'Unsafe archive member');
      assert.doesNotMatch(segment, /^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/i, 'Reserved archive member');
    }
    assert.ok(!seen.has(canonical.toLowerCase()), 'Duplicate archive member');
    const parts = canonical.split('/');
    for (let count = 1; count < parts.length; count += 1) {
      assert.ok(!files.has(parts.slice(0, count).join('/').toLowerCase()), 'Archive file used as a directory');
    }
    if (!directory) assert.ok(![...seen].some(previous => previous.startsWith(canonical.toLowerCase() + '/')),
      'Archive directory replaced by a file');
    seen.add(canonical.toLowerCase());
    assert.ok(seen.size <= 4096, 'Too many archive entries');
    if (directory) assert.equal(size, 0, 'Directory has a payload');
    else {
      files.add(canonical.toLowerCase());
      entries.set(canonical, bytes.subarray(position, position + size));
    }
    position += Math.ceil(size / 512) * 512;
  }
  assert.ok(ended && entries.has('package/package.json'), 'Incomplete package archive');
  return entries;
}
