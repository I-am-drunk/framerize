import assert from 'node:assert/strict';
import test from 'node:test';
import vm from 'node:vm';
import {capture} from './castle_v3_snapshot_values.mjs';

test('preserves value distinctions from another realm', () => {
  const value = vm.runInNewContext('[, undefined, null, -0, NaN, Infinity, "\\ud800", [true]]');
  const copy = capture(value);
  assert.deepEqual(copy.values.map(item => item.kind),
    ['undefined', 'undefined', 'null', 'number', 'number', 'number', 'string', 'array']);
  assert.deepEqual(copy.values.slice(3, 6).map(item => item.value), ['-0', 'NaN', 'Infinity']);
  assert.deepEqual(copy.values[6].units, [0xd800]);
  value[7][0] = false;
  assert.equal(copy.values[7].values[0].value, true);
});

test('refuses observable reads without invoking them', () => {
  let reads = 0;
  const value = [];
  Object.defineProperty(value, 0, {get() { reads++; return 1; }});
  assert.throws(() => capture(value), /collector-accessor/);
  const inherited = Object.create(Array.prototype, {0: {get() { reads++; return 1; }}});
  assert.throws(() => capture(Object.setPrototypeOf(Array(1), inherited)), /inherited-index/);
  const proxy = new Proxy([], {ownKeys() { reads++; return []; }});
  assert.throws(() => capture(proxy), /collector-data-shape/);
  assert.equal(reads, 0);
});

test('refuses unsupported objects, cycles and excessive input', () => {
  const cycle = []; cycle.push(cycle);
  for (const value of [cycle, {}, new Date(), 1n, () => {}, new Uint8Array(1)])
    assert.throws(() => capture(value), /collector-data-shape/);
  assert.throws(() => capture(Array(2049)), /collector-array-limit/);
  assert.throws(() => capture('x'.repeat(65537)), /collector-string-limit/);
  let nested = [];
  for (let i = 0; i < 65; i++) nested = [nested];
  assert.throws(() => capture(nested), /collector-capture-limit/);
  const shared = [1];
  assert.deepEqual(capture([shared, shared]).values[0], capture(shared));
});
