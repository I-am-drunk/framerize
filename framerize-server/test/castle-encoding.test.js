import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { types } from 'node:util';
import vm from 'node:vm';
import { createCastleMinter } from '../dist/castle.js';
import { collectionInputs, frame } from './helpers/castle-encoder-inputs.js';

function encoderSource(path) {
  const source = readFileSync(new URL(path, import.meta.url), 'utf8');
  const start = source.indexOf('var CastleEncoder = ');
  const end = source.indexOf('function createCastleMinter(input = {}) {');
  assert.ok(start >= 0 && end > start, 'The bundled encoder must be present');
  return source.slice(start, end);
}

test('bundled encoder matches its source receipt', () => {
  const receipt = JSON.parse(readFileSync(new URL('./helpers/castle-encoder-source.json', import.meta.url)));
  const source = encoderSource('../src/castle.ts');
  assert.equal(createHash('sha256').update(source).digest('hex'), receipt.encoderSha256);
  assert.equal(Object.keys(receipt.sources).length, 11);
});

test('compiled castle.js encoder matches source encoder output', () => {
  const context = vm.createContext({ types, Buffer, TextDecoder });
  const fromSource = vm.runInContext(encoderSource('../src/castle.ts') + '\nCastleEncoder;', context);
  const fromDist = vm.runInContext(encoderSource('../dist/castle.js') + '\nCastleEncoder;', context);
  for (const { text, timestamp } of collectionInputs()) {
    const input = frame(text);
    assert.equal(fromSource.encodeFrame(input, timestamp), fromDist.encodeFrame(input, timestamp));
  }
});

test('shipped encoder preserves all 128 immutable reference frames', () => {
  const context = vm.createContext({ types, Buffer, TextDecoder });
  const encoder = vm.runInContext(encoderSource('../dist/castle.js') + '\nCastleEncoder;', context);
  const digest = createHash('sha256');
  let count = 0;
  let bytes = 0;
  for (const { text, timestamp } of collectionInputs()) {
    const input = frame(text);
    const snapshot = Buffer.from(input);
    const token = Buffer.from(encoder.encodeFrame(input, timestamp));
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
  assert.throws(() => encoder.encodeFrame(Buffer.alloc(12)), { code: 'encoder' });
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

test('minter rejects overlapping mint calls', async context => {
  const minter = createCastleMinter();
  context.after(async () => minter.close());
  const inFlight = minter.mint({ signal: AbortSignal.timeout(9000) });
  await assert.rejects(() => minter.mint({ signal: AbortSignal.timeout(1000) }), /busy/);
  await inFlight;
});

test('minter rejects mint after close', async () => {
  const minter = createCastleMinter();
  await minter.close();
  await assert.rejects(() => minter.mint({ signal: AbortSignal.timeout(1000) }), /closed/);
});
