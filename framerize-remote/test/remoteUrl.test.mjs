import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseRemoteEndpoint } from '../dist/index.js';

test('accepts an https origin with no path', () => {
  const result = parseRemoteEndpoint('https://api.example.com', false);
  assert.equal(result.origin, 'https://api.example.com');
  assert.equal(result.loopback, false);
});

test('accepts numeric loopback over plain http', () => {
  const result = parseRemoteEndpoint('http://127.0.0.1:8789', false);
  assert.equal(result.origin, 'http://127.0.0.1:8789');
  assert.equal(result.loopback, true);
});

test('rejects plain http to a non-loopback host without consent', () => {
  assert.throws(() => parseRemoteEndpoint('http://api.example.com', false));
});

test('allows plain http to a non-loopback host with explicit consent', () => {
  const result = parseRemoteEndpoint('http://api.example.com', true);
  assert.equal(result.origin, 'http://api.example.com');
});

test('rejects embedded credentials, path, query, and fragment', () => {
  assert.throws(() => parseRemoteEndpoint('https://user:pw@api.example.com', false));
  assert.throws(() => parseRemoteEndpoint('https://api.example.com/v1', false));
  assert.throws(() => parseRemoteEndpoint('https://api.example.com/?a=1', false));
  assert.throws(() => parseRemoteEndpoint('https://api.example.com/#x', false));
});
