import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { createProvider } from '../dist/provider.js';
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

test('real provider preserves an upstream error body through the app', async (t) => {
  const events = [];
  const started = process.hrtime.bigint();
  const record = (event, detail = '') => events.push({ event, detail,
    elapsedMs: Number(process.hrtime.bigint() - started) / 1e6 });
  t.after(() => { if (!t.passed) console.error('safe integration events', JSON.stringify(events)); });
  const upstream = await upstreamFixture(t, (request, response) => {
    record('upstream.request');
    response.on('finish', () => record('upstream.finish'));
    response.on('close', () => record('upstream.close'));
    response.writeHead(429, {
      'content-type': 'application/json', 'retry-after': '7', 'x-request-id': 'safe-request-id',
    });
    response.end('{"error":{"message":"fixture-secret and private upstream detail"}}');
  });
  const config = {
    upstream, authorization: 'fixture-credential', headersTimeoutMs: 500, idleTimeoutMs: 500,
  };
  const { port, app } = await startFixture(t, {}, class {
    constructor() { this.provider = createProvider(config); }
    ready(options) { return this.provider.ready(options); }
    async forward(options) {
      record('provider.enter');
      try {
        const result = await this.provider.forward(options);
        record('provider.result', result.status);
        return result;
      } catch (error) { record('provider.error', error.code); throw error; }
    }
    close() { return this.provider.close(); }
  });
  const emit = app.server.emit;
  app.server.emit = function (event, ...args) {
    if (event === 'request') record('app.request');
    if (event === 'clientError') record('app.clientError', args[0]?.code);
    return emit.call(this, event, ...args);
  };
  t.after(() => { app.server.emit = emit; });
  app.server.on('connection', (socket) => {
    record('app.connection');
    socket.on('error', (error) => record('app.socketError', error.code));
    socket.on('close', () => record('app.socketClose'));
  });
  app.server.on('request', (request, response) => {
    response.on('finish', () => record('app.finish', response.statusCode));
    response.on('close', () => record('app.close', response.statusCode));
  });
  const { response, body } = await apiRequest(port, '/v1/responses', {
    method: 'POST', headers: authorizedHeaders(), body: '{}',
  });
  assert.equal(response.status, 429);
  assert.equal(response.headers.get('retry-after'), '7');
  assert.equal(response.headers.get('x-request-id'), 'safe-request-id');
  assert.equal(response.headers.get('content-type'), 'application/json');
  assert.deepEqual(JSON.parse(body), { error: {
    message: 'fixture-secret and private upstream detail',
  } });
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
