import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { types } from 'node:util';
import vm from 'node:vm';
import { createCastleMinter } from '../dist/castle.js';
import { collectionInputs, frame } from './helpers/castle-encoder-inputs.js';

function loadEncoder(path = '../dist/castle.js') {
  const context = vm.createContext({ types, Buffer, TextDecoder });
  return vm.runInContext(encoderSource(path) + '\nCastleEncoder;', context);
}

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

test('shipped encoder preserves all 128 immutable reference frames', () => {
  const encoder = loadEncoder();
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

test('encodeFrame rejects malformed collection frames and duplicate JSON keys', () => {
  const encoder = loadEncoder();
  const first = collectionInputs().next().value;
  const valid = frame(first.text);
  const badMagic = Buffer.from(valid);
  badMagic[0] = 0;
  assert.throws(() => encoder.encodeFrame(badMagic, first.timestamp), { code: 'encoder' });
  const badLength = Buffer.from(valid);
  badLength.writeUInt32BE(999, 8);
  assert.throws(() => encoder.encodeFrame(badLength, first.timestamp), { code: 'encoder' });
  const invalidUtf8 = Buffer.alloc(13);
  invalidUtf8.write('FCLT');
  invalidUtf8[4] = 1;
  invalidUtf8.writeUInt32BE(1, 8);
  invalidUtf8[12] = 0xff;
  assert.throws(() => encoder.encodeFrame(invalidUtf8, first.timestamp), { code: 'encoder' });
  const duplicateKeys = first.text.replace(/^\{"version":3/, '{"version":3,"version":3');
  assert.throws(() => encoder.encodeFrame(frame(duplicateKeys), first.timestamp), { code: 'encoder' });
});

test('encodeFrame preserves a pinned reference token for the first fixture frame', () => {
  const encoder = loadEncoder();
  const first = collectionInputs().next().value;
  const token = encoder.encodeFrame(frame(first.text), first.timestamp);
  assert.match(token, /^cNz8rPYH\|[A-Za-z0-9+/]+={0,2}$/);
  assert.equal(token.length, 241);
  assert.equal(createHash('sha256').update(token).digest('hex'),
    '79beca89fbc97cde94f710b3f05941bdee88f461278720145053fa99c9340f1b');
});

test('minter rejects overlapping mint calls and mint after close', async context => {
  const minter = createCastleMinter();
  context.after(() => minter.close());
  const inFlight = minter.mint({ signal: AbortSignal.timeout(9000) });
  await assert.rejects(
    () => minter.mint({ signal: AbortSignal.timeout(500) }),
    /busy/,
  );
  await inFlight;
  await minter.close();
  await assert.rejects(
    () => minter.mint({ signal: AbortSignal.timeout(500) }),
    /closed/,
  );
});

test('minter produces encoded tokens instead of array text', async context => {
  const minter = createCastleMinter();
  context.after(() => minter.close());
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const { token } = await minter.mint({ signal: AbortSignal.timeout(9000) });
    assert.ok(/^cNz8rPYH\|[A-Za-z0-9+/]+={0,2}$/.test(token), 'Expected an encoded Castle token');
    assert.ok(!token.startsWith('['), 'Encoded tokens must not look like stringified graph output');
    assert.ok(token.length <= 16384);
    const payload = token.slice('cNz8rPYH|'.length);
    assert.ok(Buffer.from(payload, 'base64').toString('base64') === payload, 'Expected canonical base64');
  }
});
