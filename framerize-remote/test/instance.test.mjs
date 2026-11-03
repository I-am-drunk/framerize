import { test } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { createInstance } from '@framerize/core';
import { createRemoteBackend } from '../dist/index.js';
import { startSyntheticRemote } from './synthetic-remote.mjs';

const timings = {
  headersTimeoutMs: 5000, idleTimeoutMs: 5000,
  readinessBodyMaxBytes: 65536, readinessWholeMs: 10000,
};

function request(port, path, headers, body) {
  return new Promise((resolve, reject) => {
    const req = http.request({ host: '127.0.0.1', port, path, method: 'POST', headers }, (res) => {
      const chunks = [];
      res.on('data', (chunk) => chunks.push(chunk));
      res.on('end', () => resolve({ status: res.statusCode, text: Buffer.concat(chunks).toString('utf8') }));
    });
    req.on('error', reject);
    req.end(body);
  });
}

test('end to end: client key replaced by server token, bytes preserved', async () => {
  const remote = await startSyntheticRemote();
  const backend = createRemoteBackend({
    endpoint: { origin: remote.origin, loopback: true }, serverToken: 'remote-server-token', ...timings,
  });
  const instance = createInstance({ backend, token: 'local-client-key', host: '127.0.0.1', port: 0 });
  const { port } = await instance.listen();
  try {
    const res = await request(port, '/v1/responses',
      { authorization: 'Bearer local-client-key', 'content-type': 'application/json' }, '{"model":"x"}');
    assert.equal(res.status, 200);
    assert.match(res.text, /"delta":"OK"/);
    assert.ok(remote.received.authorizations.every((value) => value === 'Bearer remote-server-token'));
    assert.ok(!remote.received.authorizations.includes('Bearer local-client-key'));
  } finally { await instance.close(); await remote.close(); }
});

test('end to end: a wrong local key is rejected locally and never forwarded', async () => {
  const remote = await startSyntheticRemote();
  const backend = createRemoteBackend({
    endpoint: { origin: remote.origin, loopback: true }, serverToken: 'remote-server-token', ...timings,
  });
  const instance = createInstance({ backend, token: 'local-client-key', host: '127.0.0.1', port: 0 });
  const { port } = await instance.listen();
  try {
    const res = await request(port, '/v1/responses',
      { authorization: 'Bearer wrong-key', 'content-type': 'application/json' }, '{"model":"x"}');
    assert.equal(res.status, 401);
    assert.equal(remote.received.paths.length, 0);
  } finally { await instance.close(); await remote.close(); }
});
