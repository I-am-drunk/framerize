import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {createCollector, writeCollection} from './collection.mjs';

const withReply = value => {
  const owner = createCollector();
  owner.sharedWorker(value);
  owner.network(undefined);
  return owner.collect;
};

test('each collector accepts only one complete operation', () => {
  const collect = withReply(undefined), values = Array(765);
  values[0] = 'first';
  const first = collect(values);
  values[0] = 'second';
  assert.throws(() => collect(values), /collection-boundary/);
  assert.notEqual(withReply(undefined)(values), first);
  assert.deepEqual(JSON.parse(first).values.values[0].units, [102,105,114,115,116]);
  for (const size of [0,764,766])
    assert.throws(() => withReply(undefined)(Array(size)), /collection-boundary/);
});

test('proxies and aggregate string excess are refused before transfer', () => {
  let reads = 0;
  const proxy = new Proxy(Array(765), {get() { reads++; return 765; }});
  assert.throws(() => withReply(undefined)(proxy), /collection-boundary/);
  assert.equal(reads, 0);
  const values = Array(765);
  values[0] = 'x'.repeat(32769); values[1] = values[0];
  assert.throws(() => withReply(undefined)(values), /collector-string-limit/);
});

test('private writer emits one exact frame and closes on success or refusal', () => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'collection-frame-'));
  try {
    for (const invalid of [false, true]) {
      const file = path.join(directory, String(invalid));
      const fd = fs.openSync(file, 'wx', 0o600);
      const body = withReply(undefined)(Array(765));
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

test('requires one reply owned by this operation and copies it before later mutation', () => {
  const missing = createCollector();
  assert.throws(() => missing.collect(Array(765)), /collection-worker-missing/);
  assert.throws(() => missing.sharedWorker([]), /collection-worker-boundary/);
  const owner = createCollector(), reply = [2];
  owner.network(undefined);
  reply[10] = 'version-at-receipt';
  owner.sharedWorker(reply);
  reply[10] = 'later-mutation';
  assert.throws(() => owner.sharedWorker(reply), /collection-worker-boundary/);
  const frame = JSON.parse(owner.collect(Array(765)));
  assert.equal(frame.version, 3);
  assert.equal(String.fromCharCode(...frame.shared_worker_reply.values[10].units), 'version-at-receipt');
  assert.throws(() => owner.sharedWorker(reply), /collection-worker-boundary/);
});

test('reply and final array share capture limits; rejected replies cannot be replaced', () => {
  const owner = createCollector();
  owner.network(undefined);
  owner.sharedWorker('x'.repeat(32768));
  const values = Array(765); values[0] = 'x'.repeat(32769);
  assert.throws(() => owner.collect(values), /collector-string-limit/);
  const failed = createCollector();
  assert.throws(() => failed.sharedWorker({}), /collector-data-shape/);
  assert.throws(() => failed.sharedWorker(undefined), /collection-worker-boundary/);
  assert.throws(() => failed.collect(Array(765)), /collection-worker-missing/);
});

test('network result is required once and copied before later mutation', () => {
  const missing = createCollector();
  missing.sharedWorker(undefined);
  assert.throws(() => missing.collect(Array(765)), /collection-network-missing/);
  assert.throws(() => missing.network([]), /collection-network-boundary/);
  for (const networkFirst of [false, true]) {
    const owner = createCollector(), record = [];
    record[38] = 'current-address';
    if (!networkFirst) owner.sharedWorker(undefined);
    owner.network([record]);
    if (networkFirst) owner.sharedWorker(undefined);
    record[38] = 'later-address';
    assert.throws(() => owner.network([record]), /collection-network-boundary/);
    const value = JSON.parse(owner.collect(Array(765))).network_reply.values[0].values[38];
    assert.equal(String.fromCharCode(...value.units), 'current-address');
    assert.throws(() => owner.network([]), /collection-network-boundary/);
  }
  const failed = createCollector();
  assert.throws(() => failed.network({}), /collector-data-shape/);
  assert.throws(() => failed.network(undefined), /collection-network-boundary/);
  failed.sharedWorker(undefined);
  assert.throws(() => failed.collect(Array(765)), /collection-network-missing/);
});

test('both receiver results and collected values share one size budget', () => {
  const owner = createCollector(), text = 'x'.repeat(22000);
  owner.network(text);
  owner.sharedWorker(text);
  const values = Array(765); values[0] = text;
  assert.throws(() => owner.collect(values), /collector-string-limit/);
});
