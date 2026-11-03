import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { createProvider } from '../src/provider.js';
import { apiRequest, authorizedHeaders, startFixture } from './helpers/fixtures.js';

async function upstreamFixture(t, handler) {
  const server = http.createServer(handler);
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  t.after(() => {
    server.closeAllConnections();
    return new Promise((resolve) => server.close(resolve));
  });
  return `http://127.0.0.1:${server.address().port}`;
}

test('real provider redacts an upstream error body through the app', async (t) => {
  const upstream = await upstreamFixture(t, (request, response) => {
    response.writeHead(429, {
      'content-type': 'application/json', 'retry-after': '7', 'x-request-id': 'safe-request-id',
    });
    response.end('{"error":{"message":"fixture-secret and private upstream detail"}}');
  });
  const config = {
    upstream, authorization: 'fixture-credential', headersTimeoutMs: 500, idleTimeoutMs: 500,
  };
  const { port } = await startFixture(t, {}, class {
    constructor() { this.provider = createProvider(config); }
    ready(options) { return this.provider.ready(options); }
    forward(options) { return this.provider.forward(options); }
    close() { return this.provider.close(); }
  });
  const { response, body } = await apiRequest(port, '/v1/responses', {
    method: 'POST', headers: authorizedHeaders(), body: '{}',
  });
  assert.equal(response.status, 429);
  assert.equal(response.headers.get('retry-after'), '7');
  assert.equal(response.headers.get('x-request-id'), 'safe-request-id');
  assert.deepEqual(JSON.parse(body), { error: {
    type: 'upstream_error', code: 'upstream_http_error', message: 'Upstream request failed',
  } });
  assert.equal(body.includes('fixture-secret'), false);
});

test('real provider preserves ready bytes when the event loop was blocked', async (t) => {
  let upstreamResponse;
  const upstream = await upstreamFixture(t, async (request, response) => {
    upstreamResponse = response;
    response.writeHead(200, { 'content-type': 'text/event-stream' });
    response.write('first');
  });
  const { port } = await startFixture(t, { idleTimeoutMs: 40 }, class {
    constructor() {
      this.provider = createProvider({ upstream, headersTimeoutMs: 500, idleTimeoutMs: 40 });
    }
    forward(options) { return this.provider.forward(options); }
    close() { return this.provider.close(); }
  });
  const body = await new Promise((resolve, reject) => {
    const request = http.request(`http://127.0.0.1:${port}/v1/responses`, {
      method: 'POST', headers: authorizedHeaders({ 'content-length': 2 }),
    }, (response) => {
      const chunks = [];
      response.on('data', (chunk) => {
        chunks.push(chunk);
        if (chunks.length === 1) setImmediate(() => {
          upstreamResponse.end('second');
          Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 100);
        });
      });
      response.on('end', () => resolve(Buffer.concat(chunks)));
      response.on('error', reject);
    });
    request.on('error', reject);
    request.end('{}');
  });
  assert.equal(body.toString(), 'firstsecond');
});
