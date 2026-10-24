import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { hashV2Input } from '../../scripts/offline_worker/v2_runtime.mjs';

function fixture(t, bytes) {
  const root = fs.mkdtempSync(join(tmpdir(), 'v2-hash-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const path = join(root, 'input'); fs.writeFileSync(path, bytes);
  return path;
}
const expected = bytes => createHash('sha256').update(bytes).digest('hex');

test('hashes_empty_and_partial_final_chunks_exactly', t => {
  for (const size of [0, 1, 65536, 65537, 196613]) {
    const bytes = Buffer.alloc(size, 0x6d);
    assert.equal(hashV2Input(fixture(t, bytes)), expected(bytes));
  }
});
test('detects_changed_bytes_beyond_the_first_chunk', t => {
  const bytes = Buffer.alloc(131073, 0x31), path = fixture(t, bytes);
  const original = hashV2Input(path);
  bytes[131072] ^= 1; fs.writeFileSync(path, bytes);
  assert.notEqual(hashV2Input(path), original);
  assert.equal(hashV2Input(path), expected(bytes));
});
test('large_inputs_use_bounded_reads_and_release_the_descriptor', t => {
  const bytes = Buffer.alloc(2 * 1024 * 1024 + 7, 0xa5), path = fixture(t, bytes);
  const read = fs.readSync, close = fs.closeSync;
  let reads = 0, closed;
  t.mock.method(fs, 'readFileSync', () => assert.fail('whole-file allocation'));
  t.mock.method(fs, 'readSync', (...args) => {
    assert.ok(args[1].length <= 65536); assert.ok(args[3] <= 65536);
    reads++; return read(...args);
  });
  t.mock.method(fs, 'closeSync', fd => { closed = fd; return close(fd); });
  assert.equal(hashV2Input(path), expected(bytes));
  assert.ok(reads > 32);
  assert.throws(() => fs.fstatSync(closed), { code: 'EBADF' });
});
test('read_errors_close_the_descriptor_and_propagate', t => {
  const path = fixture(t, Buffer.from('fixture')), close = fs.closeSync;
  const error = new Error('fixture-read-error'); let closed;
  t.mock.method(fs, 'readSync', () => { throw error; });
  t.mock.method(fs, 'closeSync', fd => { closed = fd; return close(fd); });
  assert.throws(() => hashV2Input(path), value => value === error);
  assert.throws(() => fs.fstatSync(closed), { code: 'EBADF' });
});
