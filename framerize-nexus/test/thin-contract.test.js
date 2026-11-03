import test from 'node:test';
import assert from 'node:assert/strict';
import { classifyReadiness } from '../src/thin/contract.js';

const CAPS = { responses: true, chatCompletions: true, streaming: true };
const READY = { apiVersion: 1, status: 'ready', capabilities: CAPS };
const bytes = (value) => new TextEncoder().encode(typeof value === 'string' ? value : JSON.stringify(value));
const state = (status, body) => classifyReadiness(status, bytes(body)).state;

test('v1 ready document with all capabilities classifies as ready', () => {
  assert.deepEqual(classifyReadiness(200, bytes(READY)), { state: 'ready' });
  assert.ok(Object.isFrozen(classifyReadiness(200, bytes(READY))));
  assert.equal(state(200, { ...READY, extra: 'ignored' }), 'ready');
});

test('v1 unavailable documents never classify as ready', () => {
  for (const reason of ['configuration_unavailable', 'credentials_unavailable', 'upstream_unavailable']) {
    const body = { apiVersion: 1, status: 'unavailable', capabilities: CAPS, reason };
    assert.deepEqual(classifyReadiness(503, bytes(body)), { state: 'unavailable', reason });
    assert.deepEqual(classifyReadiness(200, bytes(body)), { state: 'unavailable', reason });
  }
});

test('integer apiVersion other than 1 is incompatible', () => {
  for (const apiVersion of [0, 2, -1, 99]) assert.equal(state(200, { ...READY, apiVersion }), 'incompatible');
  assert.equal(state(200, { apiVersion: 2 }), 'incompatible');
});

test('invalid JSON, non-record bodies and bad versions are malformed', () => {
  for (const body of ['', '{', 'null', '[]', '"ready"', '1', 'true']) assert.equal(state(200, body), 'malformed');
  for (const apiVersion of [undefined, '1', 1.5, null, true]) assert.equal(state(200, { ...READY, apiVersion }), 'malformed');
  assert.equal(classifyReadiness(200, new Uint8Array([0xff, 0xfe])).state, 'malformed');
  for (const input of [undefined, null, '{}', JSON.stringify(READY), [1]]) {
    assert.equal(classifyReadiness(200, input).state, 'malformed');
  }
});

test('invalid document shapes are malformed', () => {
  for (const status of [undefined, 'READY', 'ok', 1]) assert.equal(state(200, { ...READY, status }), 'malformed');
  assert.equal(state(200, { ...READY, reason: 'upstream_unavailable' }), 'malformed');
  const unavailable = { apiVersion: 1, status: 'unavailable', capabilities: CAPS };
  for (const reason of [undefined, 'other', 1]) assert.equal(state(503, { ...unavailable, reason }), 'malformed');
});

test('missing or invalid capabilities are malformed; false ones are incompatible', () => {
  for (const capabilities of [undefined, null, [], true, 'all']) assert.equal(state(200, { ...READY, capabilities }), 'malformed');
  for (const name of Object.keys(CAPS)) {
    const { [name]: _omit, ...missing } = CAPS;
    assert.equal(state(200, { ...READY, capabilities: missing }), 'malformed');
    assert.equal(state(200, { ...READY, capabilities: { ...CAPS, [name]: 'true' } }), 'malformed');
    assert.equal(state(200, { ...READY, capabilities: { ...CAPS, [name]: false } }), 'incompatible');
  }
});

test('ready documents need a successful HTTP status', () => {
  for (const status of [301, 401, 404, 500, 503]) assert.equal(state(status, READY), 'unavailable');
  for (const status of [200, 204, 299]) assert.equal(state(status, READY), 'ready');
  for (const status of [undefined, '200', 99, 600, 200.5]) assert.equal(state(status, READY), 'malformed');
});

test('input bytes stay unchanged and results carry no body or credential data', () => {
  const secret = 'Bearer sk-secret-token';
  const inputs = [bytes(READY), bytes({ ...READY, token: secret }), bytes(`{"token":"${secret}"`),
    bytes({ apiVersion: 1, status: 'unavailable', capabilities: CAPS, reason: secret })];
  for (const input of inputs) {
    const copy = Uint8Array.from(input);
    const outcome = classifyReadiness(200, input);
    assert.deepEqual(input, copy);
    assert.ok(!JSON.stringify(outcome).includes('secret'));
  }
});
