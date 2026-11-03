import test from 'node:test';
import assert from 'node:assert/strict';
import {
  apiRequest, authorizedHeaders, JSON_BODY, startFixture, TOKEN,
  openSocket, rawPost, socketChunks,
} from './helpers/fixtures.js';

const readyDocument = {
  apiVersion: 1,
  status: 'ready',
  capabilities: { responses: true, chatCompletions: true, streaming: true },
};
const unauthorized = {
  error: {
    type: 'authentication_error', code: 'unauthorized', message: 'Authentication required',
  },
};

test('health exposes only local process health and API version', async (t) => {
  const { port, provider } = await startFixture(t);
  const { response, body } = await apiRequest(port, '/health');
  assert.equal(response.status, 200);
  assert.deepEqual(JSON.parse(body), { status: 'ok', apiVersion: 1 });
  assert.equal(provider.calls.length, 0);
});

test('ready returns the exact versioned capabilities and routes', async (t) => {
  const { port, provider } = await startFixture(t);
  const { response, body } = await apiRequest(port, '/ready', { headers: authorizedHeaders() });
  assert.equal(response.status, 200);
  assert.deepEqual(JSON.parse(body), {
    ...readyDocument,
  });
  assert.equal(provider.calls.length, 0);
});

test('unavailable readiness preserves API version and stable reason', async (t) => {
  const { port, provider } = await startFixture(t);
  provider.readiness = { ready: false, reason: 'upstream_unavailable' };
  const { response, body } = await apiRequest(port, '/ready', { headers: authorizedHeaders() });
  assert.equal(response.status, 503);
  assert.deepEqual(JSON.parse(body), {
    ...readyDocument, status: 'unavailable', reason: 'upstream_unavailable',
  });
});

test('readiness and inference use exact bearer authentication', async (t) => {
  const { port, provider } = await startFixture(t);
  for (const path of ['/ready', '/v1/responses']) {
    for (const authorization of [undefined, 'Bearer wrong', `Basic ${TOKEN}`]) {
      const { response, body } = await apiRequest(port, path, {
        method: path === '/ready' ? 'GET' : 'POST',
        headers: authorization ? { authorization } : {}, body: path === '/ready' ? undefined : '{}',
      });
      assert.equal(response.status, 401);
      assert.deepEqual(JSON.parse(body), unauthorized);
    }
  }
  assert.equal(provider.calls.length, 0);
});

test('both routes forward body bytes and safe headers to the provider', async (t) => {
  const { port, provider } = await startFixture(t);
  for (const path of ['/v1/responses', '/v1/chat/completions']) {
    const { response, body } = await apiRequest(port, path, {
      method: 'POST', body: JSON_BODY,
      headers: authorizedHeaders({ cookie: 'private-cookie', 'x-fixture': 'kept' }),
    });
    assert.equal(response.status, 200);
    assert.equal(response.headers.get('x-request-id'), 'fixture-response');
    assert.deepEqual(JSON.parse(body), { id: 'fixture-response' });
  }
  assert.equal(provider.calls.length, 2);
  assert.deepEqual(provider.calls.map(({ path }) => path),
    ['/v1/responses', '/v1/chat/completions']);
  for (const call of provider.calls) {
    assert.deepEqual(call.body, JSON_BODY);
    assert.equal(call.signal.aborted, false);
    assert.equal(Object.getPrototypeOf(call.headers), Object.prototype);
    assert.equal(call.headers.cookie, undefined);
  }
});

test('unknown routes and methods return distinct exact errors', async (t) => {
  const { port, provider } = await startFixture(t);
  const missing = await apiRequest(port, '/v1/missing');
  assert.equal(missing.response.status, 404);
  assert.deepEqual(JSON.parse(missing.body), {
    error: { type: 'invalid_request_error', code: 'not_found', message: 'Not found' },
  });
  const method = await apiRequest(port, '/v1/responses');
  assert.equal(method.response.status, 405);
  assert.deepEqual(JSON.parse(method.body), {
    error: { type: 'invalid_request_error', code: 'method_not_allowed', message: 'Method not allowed' },
  });
  assert.equal(provider.calls.length, 0);
});

test('malformed JSON fails before provider invocation', async (t) => {
  const { port, provider } = await startFixture(t);
  const { response, body } = await apiRequest(port, '/v1/responses', {
    method: 'POST', headers: authorizedHeaders(), body: '{"model":',
  });
  assert.equal(response.status, 400);
  assert.deepEqual(JSON.parse(body), {
    error: { type: 'invalid_request_error', code: 'invalid_json', message: 'Invalid JSON body' },
  });
  assert.equal(provider.calls.length, 0);
});

test('body limit rejects the request without invoking the provider', async (t) => {
  const { port, provider } = await startFixture(t, { maxBodyBytes: 8 });
  const { response, body } = await apiRequest(port, '/v1/responses', {
    method: 'POST', headers: authorizedHeaders(), body: '{"model":"too-large"}',
  });
  assert.equal(response.status, 413);
  assert.deepEqual(JSON.parse(body), {
    error: { type: 'invalid_request_error', code: 'body_too_large', message: 'Request body too large' },
  });
  assert.equal(provider.calls.length, 0);
});

test('missing server token denies protected routes but not health', async (t) => {
  const { port, provider } = await startFixture(t, { serverToken: '' });
  assert.equal((await apiRequest(port, '/health')).response.status, 200);
  for (const path of ['/ready', '/v1/responses']) {
    const { response, body } = await apiRequest(port, path, {
      method: path === '/ready' ? 'GET' : 'POST',
      headers: authorizedHeaders(), body: path === '/ready' ? undefined : '{}',
    });
    assert.equal(response.status, 401);
    assert.deepEqual(JSON.parse(body), unauthorized);
  }
  assert.equal(provider.calls.length, 0);
});

test('Connection-nominated safe headers do not reach the provider', async (t) => {
  const { port, provider } = await startFixture(t);
  const socket = await openSocket(port);
  t.after(() => socket.destroy());
  const received = socketChunks(socket);
  const headers = rawPost().replace('Connection: close',
    'Connection: close, accept, content-type, openai-beta');
  socket.write(headers + JSON_BODY);
  await received.closed;
  assert.match(received.text(), /^HTTP\/1\.1 200/);
  assert.equal(provider.calls[0].headers.accept, undefined);
  assert.equal(provider.calls[0].headers['content-type'], undefined);
  assert.equal(provider.calls[0].headers['openai-beta'], undefined);
});
