import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { createCastleMinter, encodeFrame } from '../dist/castle.js';
import { collectionInputs, frame } from './helpers/castle-encoder-inputs.js';

test('bundled library matches its source receipt and original encoder inputs', () => {
  const receipt = JSON.parse(readFileSync(new URL('./helpers/castle-encoder-source.json', import.meta.url)));
  const library = JSON.parse(readFileSync(new URL('./helpers/castle-library-source.json', import.meta.url)));
  const source = readFileSync(new URL('../src/castle.ts', import.meta.url));
  assert.equal(createHash('sha256').update(source).digest('hex'), library.sha256);
  const generator = readFileSync(new URL('../../scripts/vendor-castle-library.mjs', import.meta.url));
  assert.equal(createHash('sha256').update(generator).digest('hex'), library.generatorSha256);
  for (const [path, hash] of Object.entries(receipt.sources)) assert.equal(library.sources[path], hash);
  assert.equal(Object.keys(receipt.sources).length, 11);
});

test('shipped encoder preserves all 128 immutable reference frames', () => {
  const digest = createHash('sha256');
  let count = 0;
  let bytes = 0;
  for (const { text, timestamp } of collectionInputs()) {
    const input = frame(text);
    const snapshot = Buffer.from(input);
    const token = Buffer.from(encodeFrame(input, timestamp));
    const length = Buffer.alloc(4);
    length.writeUInt32BE(token.length);
    digest.update(length).update(token);
    bytes += token.length + 4;
    count += 1;
    assert.deepEqual(input, snapshot);
  }
  assert.equal(count, 128);
  assert.equal(bytes, 34520);
  assert.equal(digest.digest('hex'), '13846785e315241b3cec3152555acab063540942b370ae41551afa691b4eaab4');
  assert.throws(() => encodeFrame(Buffer.alloc(12)), { code: 'encoder' });
});

test('minter produces encoded tokens instead of array text', async context => {
  const minter = createCastleMinter();
  context.after(() => minter.close());
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const { token } = await minter.mint({ signal: AbortSignal.timeout(9000) });
    assert.ok(/^cNz8rPYH\|[A-Za-z0-9+/]+={0,2}$/.test(token), 'Expected an encoded Castle token');
    assert.ok(token.length <= 16384);
    const payload = token.slice('cNz8rPYH|'.length);
    assert.ok(Buffer.from(payload, 'base64').toString('base64') === payload, 'Expected canonical base64');
  }
});
