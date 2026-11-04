import assert from 'node:assert/strict';
import { once } from 'node:events';
import { createServer } from 'node:http';
import test from 'node:test';
import { createRemoteBackend } from '../dist/index.js';

const ready = { apiVersion: 1, status: 'ready',
  capabilities: { responses: true, chatCompletions: true, streaming: true } };

async function peer(handler) {
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
    async close() {
      server.closeAllConnections();
      await new Promise((resolve) => server.close(resolve));
    } };
}

function backendFor(remote, timings = {}) {
  return createRemoteBackend({
    endpoint: { origin: remote.origin, loopback: true }, serverToken: 'synthetic-remote-token',
    headersTimeoutMs: 2000, idleTimeoutMs: 1000, readinessWholeMs: 2000,
    readinessBodyMaxBytes: 65536, inferenceTimeoutMs: 400, ...timings,
  });
}

function forward(backend, path, extraHeaders = {}) {
  return backend.forward({
    path, headers: { 'content-type': 'application/json', ...extraHeaders },
    signal: new AbortController().signal,
    body: (async function* () { yield Buffer.from('{"model":"fixture"}'); })(),
  });
}

async function collect(body) {
  const chunks = [];
  for await (const chunk of body) chunks.push(chunk);
  return Buffer.concat(chunks);
}

test('forward rejects unknown inference paths before contacting the remote', async () => {
  const remote = await peer((incoming, outgoing) => outgoing.end('must not run'));
  const backend = backendFor(remote);
  try {
    await assert.rejects(forward(backend, '/v1/unknown'), /invalid inference path/);
    assert.deepEqual(remote.paths, []);
  } finally { backend.close(); await remote.close(); }
});

for (const path of ['/openai/v1/responses', '/anthropic/v1/messages']) {
  test(`completed ${path} skips readiness and buffers a JSON reply`, async () => {
    const payload = Buffer.from('{"opaque":"completed-reply"}');
    const remote = await peer((incoming, outgoing) => {
      if (incoming.url === '/ready') return outgoing.end('readiness must not run');
      outgoing.writeHead(200, { 'content-type': 'application/json', 'content-length': payload.length });
      outgoing.end(payload);
    });
    const backend = backendFor(remote);
    try {
      const response = await forward(backend, path);
      assert.equal(response.status, 200);
      assert.deepEqual(await collect(response.body), payload);
      assert.deepEqual(remote.paths, [path]);
    } finally { backend.close(); await remote.close(); }
  });
}

test('custom routes forward Anthropic protocol headers but /v1 routes do not', async () => {
  const remote = await peer((incoming, outgoing) => {
    if (incoming.url === '/ready') {
      outgoing.writeHead(200, { 'content-type': 'application/json' });
      return outgoing.end(JSON.stringify(ready));
    }
    outgoing.writeHead(200, { 'content-type': 'text/event-stream' });
    outgoing.end('data: [DONE]\n\n');
  });
  const backend = backendFor(remote);
  const anthropic = { 'anthropic-version': '2026-10-21', 'anthropic-beta': 'messages-2024-01-01' };
  const headers = { ...anthropic, cookie: 'synthetic-private-cookie',
    'x-api-key': 'synthetic-private-key', authorization: 'Bearer synthetic-local-token' };
  try {
    const streaming = await forward(backend, '/v1/responses', headers);
    assert.equal(streaming.status, 200);
    assert.equal((await collect(streaming.body)).toString(), 'data: [DONE]\n\n');
    assert.deepEqual(remote.paths, ['/ready', '/v1/responses']);
    assert.equal(remote.headers.at(-1)['anthropic-version'], undefined);
    assert.equal(remote.headers.at(-1)['anthropic-beta'], undefined);
    assert.equal(remote.headers.at(-1).cookie, undefined);
    assert.equal(remote.headers.at(-1)['x-api-key'], undefined);
    assert.equal(remote.headers.at(-1).authorization, 'Bearer synthetic-remote-token');
    remote.paths.length = 0;
    remote.headers.length = 0;
    const custom = await forward(backend, '/anthropic/v1/messages', headers);
    assert.equal(custom.status, 200);
    assert.equal((await collect(custom.body)).toString(), 'data: [DONE]\n\n');
    assert.equal(remote.headers.at(-1)['anthropic-version'], '2026-10-21');
    assert.equal(remote.headers.at(-1)['anthropic-beta'], 'messages-2024-01-01');
    assert.equal(remote.headers.at(-1).cookie, undefined);
    assert.equal(remote.headers.at(-1)['x-api-key'], undefined);
    assert.equal(remote.headers.at(-1).authorization, 'Bearer synthetic-remote-token');
    assert.deepEqual(remote.paths, ['/anthropic/v1/messages']);
  } finally { backend.close(); await remote.close(); }
});

test('completed inference deadlines map to non-retryable header timeouts', { timeout: 3000 }, async () => {
  const remote = await peer((incoming, outgoing) => {
    if (incoming.url === '/ready') return outgoing.end(JSON.stringify(ready));
    outgoing.flushHeaders();
  });
  const backend = backendFor(remote, { inferenceTimeoutMs: 80 });
  try {
    await assert.rejects(forward(backend, '/openai/v1/responses'), (error) => {
      assert.equal(error.message, 'Remote completed request failed');
      assert.equal(error.code, 'headers_timeout');
      assert.equal(error.status, 504);
      assert.equal(error.retryable, false);
      return true;
    });
    assert.deepEqual(remote.paths, ['/openai/v1/responses']);
  } finally { backend.close(); await remote.close(); }
});
