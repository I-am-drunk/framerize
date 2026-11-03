import test from 'node:test';
import assert from 'node:assert/strict';
import {
  apiRequest, authorizedHeaders, controlledStream, deferred,
  delay, JSON_BODY, openSocket, rawPost, socketChunks, startFixture, waitFor,
} from './helpers/fixtures.js';

test('short streams preserve bytes, status and allowlisted headers', async (t) => {
  const { port, provider } = await startFixture(t);
  provider.forwardImpl = () => ({
    status: 201,
    headers: { 'content-type': 'text/event-stream', 'x-request-id': 'short-stream' },
    body: [Buffer.from('event: response.completed\ndata: {"ok":true}\n\n')],
  });
  const { response, body } = await apiRequest(port, '/v1/responses', {
    method: 'POST', headers: authorizedHeaders(), body: JSON_BODY,
  });
  assert.equal(response.status, 201);
  assert.equal(response.headers.get('content-type'), 'text/event-stream');
  assert.equal(response.headers.get('x-request-id'), 'short-stream');
  assert.equal(body.toString(), 'event: response.completed\ndata: {"ok":true}\n\n');
});

test('long streams preserve every chunk in order', async (t) => {
  const { port, provider } = await startFixture(t);
  const chunks = Array.from({ length: 300 }, (_, index) => (
    Buffer.from(`data: ${String(index).padStart(4, '0')}:${'x'.repeat(512)}\n\n`)
  ));
  provider.forwardImpl = () => ({
    status: 200, headers: { 'content-type': 'text/event-stream' }, body: chunks,
  });
  const { response, body } = await apiRequest(port, '/v1/chat/completions', {
    method: 'POST', headers: authorizedHeaders(), body: JSON_BODY,
  });
  assert.equal(response.status, 200);
  assert.deepEqual(body, Buffer.concat(chunks));
});

test('fragmented multibyte UTF-8 is not decoded or re-encoded', async (t) => {
  const { port, provider } = await startFixture(t);
  const expected = Buffer.from('data: {"text":"A café 東京 😀"}\n\n', 'utf8');
  const chunks = Array.from(expected, (byte) => Buffer.from([byte]));
  provider.forwardImpl = () => ({
    status: 200, headers: { 'content-type': 'text/event-stream; charset=utf-8' }, body: chunks,
  });
  const { body } = await apiRequest(port, '/v1/responses', {
    method: 'POST', headers: authorizedHeaders(), body: JSON_BODY,
  });
  assert.deepEqual(body, expected);
});

test('slow consumers apply backpressure without buffering the whole stream', async (t) => {
  const { port, provider } = await startFixture(t);
  const chunk = Buffer.alloc(16 * 1024, 7);
  let produced = 0;
  let cancelled = false;
  const stream = (async function* () {
    try {
      for (; produced < 1_000; produced += 1) yield chunk;
    } finally { cancelled = true; }
  }());
  provider.forwardImpl = () => ({
    status: 200, headers: { 'content-type': 'application/octet-stream' }, body: stream,
  });
  const socket = await openSocket(port);
  t.after(() => socket.destroy());
  const received = socketChunks(socket);
  socket.write(rawPost() + JSON_BODY);
  socket.pause();
  await waitFor(() => produced > 0, 'initial stream output');
  await delay(100);
  const stabilizedAt = produced;
  await delay(40);
  assert.equal(produced, stabilizedAt);
  assert.ok(produced < 1_000, 'provider must not finish while consumer is paused');
  socket.resume();
  await waitFor(() => produced > stabilizedAt, 'producer to resume');
  socket.destroy();
  await waitFor(() => cancelled, 'stream iterator cleanup');
});

test('incomplete HTTP headers time out before provider invocation', async (t) => {
  const { port, provider } = await startFixture(t, {
    headersTimeoutMs: 60, requestHeadersTimeoutMs: 40,
  });
  const socket = await openSocket(port);
  t.after(() => socket.destroy());
  const received = socketChunks(socket);
  socket.write(`POST /v1/responses HTTP/1.1\r\nHost: local\r\nAuthorization: Bearer `);
  await Promise.race([received.closed, received.failed]);
  assert.ok(received.text().startsWith('HTTP/1.1 408'));
  assert.equal(provider.calls.length, 0);
});

test('provider header stalls abort and return an exact error', async (t) => {
  const { port, provider } = await startFixture(t, { headersTimeoutMs: 40 });
  let aborts = 0;
  provider.forwardImpl = ({ signal }) => {
    signal.addEventListener('abort', () => { aborts += 1; }, { once: true });
    return new Promise(() => {});
  };
  const { response, body } = await apiRequest(port, '/v1/responses', {
    method: 'POST', headers: authorizedHeaders(), body: JSON_BODY,
  });
  assert.equal(response.status, 504);
  assert.deepEqual(JSON.parse(body), {
    error: { type: 'invalid_request_error', code: 'headers_timeout', message: 'Upstream headers timed out' },
  });
  assert.equal(aborts, 1);
});

test('idle body stalls truncate the response and cancel iteration', async (t) => {
  const { port, provider } = await startFixture(t, { idleTimeoutMs: 50 });
  let cleaned = false;
  const source = controlledStream();
  provider.forwardImpl = ({ signal }) => ({
    status: 200, headers: { 'content-type': 'text/event-stream' },
    body: { async *[Symbol.asyncIterator]() {
      signal.addEventListener('abort', () => source.fail(signal.reason), { once: true });
      try { yield* source; } finally { cleaned = true; }
    } },
  });
  const socket = await openSocket(port);
  t.after(() => socket.destroy());
  const received = socketChunks(socket);
  socket.write(rawPost() + JSON_BODY);
  source.write('data: started\n\n');
  await received.waitFor('data: started');
  await Promise.race([received.closed, received.failed]);
  assert.equal(received.text().includes('[DONE]'), false);
  assert.equal(received.text().includes('0\r\n\r\n'), false);
  await waitFor(() => cleaned, 'idle stream iterator cleanup');
});

test('active streams have no total request deadline', async (t) => {
  const { port, provider } = await startFixture(t, {
    headersTimeoutMs: 30, idleTimeoutMs: 500,
  });
  const stream = controlledStream();
  provider.forwardImpl = () => ({
    status: 200, headers: { 'content-type': 'text/event-stream' }, body: stream,
  });
  const pending = apiRequest(port, '/v1/responses', {
    method: 'POST', headers: authorizedHeaders(), body: JSON_BODY,
  });
  stream.write('data: first\n\n');
  await delay(180);
  stream.write('data: last\n\n');
  stream.end();
  const { response, body } = await pending;
  assert.equal(response.status, 200);
  assert.equal(body.toString(), 'data: first\n\ndata: last\n\n');
});

test('disconnect aborts provider and closes its stream iterator', async (t) => {
  const { port, provider } = await startFixture(t);
  const aborted = deferred();
  const cleaned = deferred();
  const stream = controlledStream();
  provider.forwardImpl = ({ signal }) => {
    signal.addEventListener('abort', () => {
      aborted.resolve(signal.reason);
      stream.fail(signal.reason);
    }, { once: true });
    return {
      status: 200, headers: { 'content-type': 'text/event-stream' },
      body: { async *[Symbol.asyncIterator]() {
        try { yield* stream; } finally { cleaned.resolve(); }
      } },
    };
  };
  const socket = await openSocket(port);
  t.after(() => socket.destroy());
  const received = socketChunks(socket);
  socket.write(rawPost() + JSON_BODY);
  stream.write('data: connected\n\n');
  await received.waitFor('data: connected');
  socket.destroy();
  await Promise.all([aborted.promise, cleaned.promise]);
  assert.equal(provider.calls[0].signal.aborted, true);
});

test('failed streams truncate without a successful terminal marker', async (t) => {
  const { port, provider } = await startFixture(t);
  const stream = controlledStream();
  provider.forwardImpl = () => ({
    status: 200, headers: { 'content-type': 'text/event-stream' }, body: stream,
  });
  const socket = await openSocket(port);
  t.after(() => socket.destroy());
  const received = socketChunks(socket);
  socket.write(rawPost() + JSON_BODY);
  stream.write('data: partial\n\n');
  await received.waitFor('data: partial');
  stream.fail(new Error('fixture upstream reset'));
  await Promise.race([received.closed, received.failed]);
  assert.equal(received.text().includes('response.completed'), false);
  assert.equal(received.text().includes('[DONE]'), false);
  assert.equal(received.text().includes('0\r\n\r\n'), false);
});

test('non-2xx provider bodies are safe OpenAI errors with original status', async (t) => {
  const { port, provider } = await startFixture(t);
  provider.forwardImpl = () => ({
    status: 429,
    headers: { 'content-type': 'application/json', 'retry-after': '2', 'x-request-id': 'rate-limit' },
    body: [Buffer.from(JSON.stringify({
      error: {
        type: 'upstream_error', code: 'upstream_http_error',
        message: 'Upstream request failed',
      },
    }))],
  });
  const { response, body } = await apiRequest(port, '/v1/responses', {
    method: 'POST', headers: authorizedHeaders(), body: JSON_BODY,
  });
  assert.equal(response.status, 429);
  assert.equal(response.headers.get('retry-after'), '2');
  assert.deepEqual(JSON.parse(body), {
    error: {
      type: 'upstream_error', code: 'upstream_http_error', message: 'Upstream request failed',
    },
  });
  assert.equal(body.includes('fixture-secret'), false);
});
