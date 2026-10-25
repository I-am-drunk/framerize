import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {createCollector, writeCollection} from './collection.mjs';

test('each collector accepts only one complete operation', () => {
  const collect = createCollector(), values = Array(765);
  values[0] = 'first';
  const first = collect(values);
  values[0] = 'second';
  assert.throws(() => collect(values), /collection-boundary/);
  assert.notEqual(createCollector()(values), first);
  assert.deepEqual(JSON.parse(first).values.values[0].units, [102,105,114,115,116]);
  for (const size of [0,764,766])
    assert.throws(() => createCollector()(Array(size)), /collection-boundary/);
});

test('proxies and aggregate string excess are refused before transfer', () => {
  let reads = 0;
  const proxy = new Proxy(Array(765), {get() { reads++; return 765; }});
  assert.throws(() => createCollector()(proxy), /collection-boundary/);
  assert.equal(reads, 0);
  const values = Array(765);
  values[0] = 'x'.repeat(32769); values[1] = values[0];
  assert.throws(() => createCollector()(values), /collector-string-limit/);
});

test('private writer emits one exact frame and closes on success or refusal', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'collection-frame-'));
  try {
    for (const invalid of [false, true]) {
      const file = path.join(directory, String(invalid));
      const fd = fs.openSync(file, 'wx', 0o600);
      const body = createCollector()(Array(765));
      if (invalid) assert.throws(() => writeCollection(fd, ''), /collection-value/);
      else writeCollection(fd, body);
      assert.throws(() => fs.fstatSync(fd), {code: 'EBADF'});
      const bytes = fs.readFileSync(file);
      if (invalid) assert.equal(bytes.length, 0);
      else {
        assert.deepEqual(bytes.subarray(0, 8), Buffer.from([70,67,76,84,1,0,0,0]));
        assert.equal(bytes.readUInt32BE(8), Buffer.byteLength(body));
        assert.equal(bytes.subarray(12).toString(), body);
      }
    }
  } finally { fs.rmSync(directory, {recursive: true, force: true}); }
});
