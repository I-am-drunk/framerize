import { test } from 'node:test';
import assert from 'node:assert/strict';
import { once } from 'node:events';
import { createServer } from 'node:http';
import { createRemoteBackend } from '../dist/index.js';
import { startSyntheticRemote } from './synthetic-remote.mjs';

const timings = {
  headersTimeoutMs: 5000, idleTimeoutMs: 5000,
  readinessBodyMaxBytes: 64 * 1024, readinessWholeMs: 10000,
};

function backendFor(remote, token = 'remote-server-token') {
  return createRemoteBackend({
    endpoint: { origin: remote.origin, loopback: true },
    serverToken: token, ...timings,
  });
}

async function collect(body) {
  const chunks = [];
  for await (const chunk of body) chunks.push(chunk);
  return Buffer.concat(chunks).toString('utf8');
}

test('ready returns true for a contract-v1 ready remote', async () => {
  const remote = await startSyntheticRemote();
  const backend = backendFor(remote);
  try {
    const result = await backend.ready({ signal: new AbortController().signal });
    assert.equal(result, true);
    assert.deepEqual(remote.received.authorizations, ['Bearer remote-server-token']);
  } finally { backend.close(); await remote.close(); }
});

test('ready maps an unavailable document to its reason', async () => {
  const remote = await startSyntheticRemote({ ready: { apiVersion: 1, status: 'unavailable',
    reason: 'upstream_unavailable', capabilities: { responses: true, chatCompletions: true, streaming: true } } });
  const backend = backendFor(remote);
  try {
    const result = await backend.ready({ signal: new AbortController().signal });
    assert.deepEqual(result, { ready: false, reason: 'upstream_unavailable' });
  } finally { backend.close(); await remote.close(); }
});

test('forward streams bytes unchanged and sends the server token only', async () => {
  const remote = await startSyntheticRemote();
  const backend = backendFor(remote);
  try {
    const body = (async function* () { yield Buffer.from('{"model":"x"}'); })();
    const response = await backend.forward({ path: '/v1/responses',
      headers: { 'content-type': 'application/json' }, body, signal: new AbortController().signal });
    assert.equal(response.status, 200);
    const text = await collect(response.body);
    assert.match(text, /"delta":"OK"/);
    assert.match(text, /\[DONE\]/);
    assert.ok(remote.received.authorizations.every((value) => value === 'Bearer remote-server-token'));
  } finally { backend.close(); await remote.close(); }
});

async function completedPeer(handler) {
  const paths = [];
  const headers = [];
  const server = createServer((request, response) => {
    paths.push(request.url);
    headers.push({ ...request.headers });
    request.resume();
    handler(request, response);
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return { paths, headers, origin: `http://127.0.0.1:${server.address().port}`,
    close() { return new Promise((resolve) => server.close(resolve)); } };
}

function completedRequest(path, extraHeaders = {}) {
  return { path, headers: { 'content-type': 'application/json', ...extraHeaders },
    signal: new AbortController().signal,
    body: (async function* () { yield Buffer.from('{"model":"fixture"}'); })() };
}

for (const path of ['/openai/v1/responses', '/anthropic/v1/messages']) {
  test(`completed ${path} skips readiness and preserves opaque bytes`, async () => {
    const remote = await completedPeer((incoming, outgoing) => {
      if (incoming.url === '/ready') outgoing.end('readiness must not run');
      else outgoing.end('opaque-completed-body');
    });
    const backend = backendFor({ origin: remote.origin, received: { authorizations: [] } });
    try {
      const response = await backend.forward(completedRequest(path));
      assert.equal(response.status, 200);
      assert.deepEqual(response.body, [Buffer.from('opaque-completed-body')]);
      assert.deepEqual(remote.paths, [path]);
    } finally { backend.close(); await remote.close(); }
  });
}

test('completed Anthropic routes forward protocol headers to the server', async () => {
  const remote = await completedPeer((_incoming, outgoing) => outgoing.end('{}'));
  const backend = backendFor({ origin: remote.origin, received: { authorizations: [] } });
  try {
    await backend.forward(completedRequest('/anthropic/v1/messages', {
      'anthropic-version': '2026-10-21', 'anthropic-beta': 'synthetic-beta', cookie: 'private',
    }));
    const forwarded = remote.headers[0];
    assert.equal(forwarded['anthropic-version'], '2026-10-21');
    assert.equal(forwarded['anthropic-beta'], 'synthetic-beta');
    assert.equal(forwarded.cookie, undefined);
    assert.equal(forwarded.authorization, 'Bearer remote-server-token');
  } finally { backend.close(); await remote.close(); }
});

test('forward rejects unsupported inference paths before contacting the server', async () => {
  const remote = await completedPeer(() => {});
  const backend = backendFor({ origin: remote.origin, received: { authorizations: [] } });
  try {
    await assert.rejects(backend.forward(completedRequest('/vendor/v1/completions')),
      /invalid inference path/);
    assert.deepEqual(remote.paths, []);
  } finally { backend.close(); await remote.close(); }
});

test('completed route body timeouts map to non-retryable upstream failures', async () => {
  const remote = await completedPeer((_incoming, outgoing) => {
    outgoing.writeHead(200, { 'content-type': 'application/json' });
  });
  const backend = createRemoteBackend({
    endpoint: { origin: remote.origin, loopback: true }, serverToken: 'remote-server-token',
    headersTimeoutMs: 5000, idleTimeoutMs: 50, inferenceTimeoutMs: 80, readinessWholeMs: 5000,
    readinessBodyMaxBytes: 64 * 1024,
  });
  try {
    await assert.rejects(backend.forward(completedRequest('/openai/v1/responses')), error => {
      assert.equal(error.code, 'headers_timeout');
      assert.equal(error.status, 504);
      assert.equal(error.retryable, false);
      return true;
    });
    assert.deepEqual(remote.paths, ['/openai/v1/responses']);
  } finally { backend.close(); await remote.close(); }
});
