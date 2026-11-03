import assert from 'node:assert/strict';
import test from 'node:test';
import { classifyReadiness } from '../src/index.js';

const capabilities = { responses: true, chatCompletions: true, streaming: true };
const ready = { apiVersion: 1, status: 'ready', capabilities };
const encode = (value: unknown): Uint8Array =>
  new TextEncoder().encode(JSON.stringify(value));

test('ready requires successful status and all capabilities', () => {
  for (const status of [200, 201, 204, 299]) {
    assert.deepEqual(classifyReadiness(status, encode(ready)), { state: 'ready' });
  }
  for (const status of [100, 199, 300, 401, 503, 599]) {
    assert.deepEqual(classifyReadiness(status, encode(ready)), { state: 'unavailable' });
  }
  for (const name of Object.keys(capabilities)) {
    const document = { ...ready, capabilities: { ...capabilities, [name]: false } };
    assert.deepEqual(classifyReadiness(200, encode(document)), { state: 'incompatible' });
  }
});

test('unavailable responses retain only allowlisted reasons', () => {
  for (const reason of ['configuration_unavailable', 'credentials_unavailable', 'upstream_unavailable']) {
    const document = { ...ready, status: 'unavailable', reason };
    assert.deepEqual(classifyReadiness(503, encode(document)), { state: 'unavailable', reason });
  }
  const document = { ...ready, status: 'unavailable', reason: 'SYNTHETIC_CANARY' };
  assert.deepEqual(classifyReadiness(503, encode(document)), { state: 'malformed' });
});

test('unknown runtime input is rejected without coercion', () => {
  const hostile = { toString() { throw new Error('SYNTHETIC_CANARY'); } };
  for (const status of [undefined, null, hostile, '200', NaN, Infinity, 99, 600, 200.5]) {
    assert.deepEqual(classifyReadiness(status, encode(ready)), { state: 'malformed' });
  }
  for (const body of [undefined, null, hostile, '{}', [1, 2], new Uint8Array([255])]) {
    assert.deepEqual(classifyReadiness(200, body), { state: 'malformed' });
  }
  for (const body of [null, [], 5, {}, { ...ready, reason: null }]) {
    assert.deepEqual(classifyReadiness(200, encode(body)), { state: 'malformed' });
  }
});

test('classification does not mutate input or retain opaque body data', () => {
  const body = encode({ ...ready, private: 'SYNTHETIC_CANARY' });
  const before = body.slice();
  const result = classifyReadiness(200, body);
  assert.deepEqual(body, before);
  assert.deepEqual(result, { state: 'ready' });
  assert.equal(Object.isFrozen(result), true);
  assert.equal(JSON.stringify(result).includes('SYNTHETIC_CANARY'), false);
  assert.deepEqual(classifyReadiness(200, encode({ ...ready, apiVersion: 2 })), {
    state: 'incompatible',
  });
});

test('TypeScript classifier preserves the accepted JavaScript behavior', async () => {
  const legacyUrl = new URL('../../test/fixtures/legacy-contract.mjs', import.meta.url);
  const legacy = await import(legacyUrl.href) as { classifyReadiness: typeof classifyReadiness };
  const documents: unknown[] = [null, [], {}, ready, { ...ready, reason: '' }];
  for (const version of [-1, 0, 2, 1.5, '1', null]) {
    documents.push({ ...ready, apiVersion: version });
  }
  for (const name of Object.keys(capabilities)) {
    for (const value of [false, undefined, null, 'true', 1]) {
      documents.push({ ...ready, capabilities: { ...capabilities, [name]: value } });
    }
  }
  for (const reason of ['configuration_unavailable', 'credentials_unavailable', 'upstream_unavailable', 'other']) {
    documents.push({ ...ready, status: 'unavailable', reason });
  }
  const bodies = documents.map(encode).concat([new Uint8Array([255]), new Uint8Array()]);
  for (const status of [undefined, null, '200', NaN, 99, 100, 199, 200, 204, 299, 300, 401, 503, 599, 600]) {
    for (const body of bodies) {
      assert.deepEqual(classifyReadiness(status, body), legacy.classifyReadiness(status, body));
    }
  }
});
