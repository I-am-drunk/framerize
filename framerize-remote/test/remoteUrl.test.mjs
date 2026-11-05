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

test('allows plain http to LAN and Tailscale servers without consent', () => {
  for (const origin of ['http://100.64.0.10:8787', 'http://192.168.1.20:8787', 'http://10.0.0.2',
    'http://desktop.tail1234.ts.net:8787', 'http://desktop:8787', 'http://[fd7a:115c:a1e0::1]:8787']) {
    assert.equal(parseRemoteEndpoint(origin, false).origin, origin);
  }
  for (const origin of ['http://100.128.0.1', 'http://8.8.8.8', 'http://api.example.com']) {
    assert.throws(() => parseRemoteEndpoint(origin, false));
  }
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
