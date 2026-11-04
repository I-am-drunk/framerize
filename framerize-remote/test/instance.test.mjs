import { test } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { once } from 'node:events';
import { setTimeout as delay } from 'node:timers/promises';
import { createInstance } from '@framerize/core';
import { createRemoteBackend } from '../dist/index.js';
import { startSyntheticRemote } from './synthetic-remote.mjs';

const readinessWholeMs = 20_000;
const profileInferenceMs = 1000;
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

function exchange(port, token) {
  return new Promise((resolve, reject) => {
    const req = http.request({ host: '127.0.0.1', port, path: '/v1/responses', method: 'POST', agent: false,
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' } }, (res) => {
      const chunks = [];
      res.on('data', (chunk) => chunks.push(chunk));
      res.on('end', () => resolve({ status: res.statusCode, headers: res.headers,
        body: Buffer.concat(chunks).toString('utf8') }));
    });
    req.on('error', reject);
    req.end('{"model":"fixture","stream":false}');
  });
}

function serveInstance(backend, localToken, profileInferenceTimeoutMs = profileInferenceMs) {
  return createInstance({
    backend, token: localToken, host: '127.0.0.1', port: 0,
    headersTimeoutMs: readinessWholeMs,
    inferenceTimeoutMs: readinessWholeMs + profileInferenceTimeoutMs,
    idleTimeoutMs: 60_000,
  });
}

async function jsonPeer({ token = 'remote-server-token', onInference }) {
  const ready = { apiVersion: 1, status: 'ready',
    capabilities: { responses: true, chatCompletions: true, streaming: true } };
  const server = http.createServer((incoming, outgoing) => {
    incoming.resume();
    if (incoming.url === '/ready') {
      outgoing.end(JSON.stringify(ready));
      return;
    }
    Promise.resolve(onInference(incoming, outgoing)).catch(() => outgoing.destroy());
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return { origin: `http://127.0.0.1:${server.address().port}`,
    async close() {
      server.closeAllConnections();
      await new Promise((resolve) => server.close(resolve));
    } };
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

test('serve wiring keeps readiness inside the core inference budget', async () => {
  const peer = await jsonPeer({ async onInference(_incoming, outgoing) {
    await delay(180);
    outgoing.end('{"done":true}');
  } });
  const backend = createRemoteBackend({
    endpoint: { origin: peer.origin, loopback: true }, serverToken: 'remote-server-token',
    headersTimeoutMs: 50, idleTimeoutMs: 50, inferenceTimeoutMs: profileInferenceMs,
    readinessBodyMaxBytes: 65536, readinessWholeMs,
  });
  const instance = serveInstance(backend, 'local-client-key');
  const { port } = await instance.listen();
  try {
    const result = await exchange(port, 'local-client-key');
    assert.equal(result.status, 200);
    assert.equal(result.body, '{"done":true}');
  } finally { await instance.close(); await peer.close(); }
});

test('serve wiring surfaces inference deadlines with a no-retry header', async () => {
  const peer = await jsonPeer({ async onInference(_incoming, outgoing) {
    await new Promise(() => {});
    outgoing.end('never');
  } });
  const backend = createRemoteBackend({
    endpoint: { origin: peer.origin, loopback: true }, serverToken: 'remote-server-token',
    headersTimeoutMs: 15_000, idleTimeoutMs: 60_000, inferenceTimeoutMs: 80,
    readinessBodyMaxBytes: 65536, readinessWholeMs,
  });
  const instance = serveInstance(backend, 'local-client-key', 80);
  const { port } = await instance.listen();
  try {
    const result = await exchange(port, 'local-client-key');
    assert.equal(result.status, 504);
    assert.equal(result.headers['x-should-retry'], 'false');
    assert.match(result.body, /headers_timeout/);
  } finally { await instance.close(); await peer.close(); }
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
