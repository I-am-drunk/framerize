import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from '../dist/server.js';
import { apiRequest, authorizedHeaders, TOKEN } from './helpers/fixtures.js';

async function instance(context, provider, serverToken = TOKEN) {
  const app = await createServer({ host: '127.0.0.1', port: 0,
    serverToken, upstream: '', accountsFile: '' }, { provider });
  context.after(() => app.close());
  const { port } = await app.listen();
  return { app, port };
}

test('missing provider returns stable unavailable responses', async context => {
  const { port } = await instance(context);
  const ready = await apiRequest(port, '/ready', { headers: authorizedHeaders() });
  assert.equal(ready.response.status, 503);
  assert.equal(JSON.parse(ready.body).reason, 'configuration_unavailable');
  const response = await apiRequest(port, '/v1/responses', {
    method: 'POST', headers: authorizedHeaders(), body: '{}',
  });
  assert.equal(response.response.status, 503);
  assert.equal(JSON.parse(response.body).error.code, 'provider_unavailable');
});

test('an unset server token permits trusted readiness and inference', async context => {
  let calls = 0;
  const { port } = await instance(context, {
    ready() { calls += 1; return true; },
    forward(request) {
      calls += 1;
      assert.equal(request.headers.authorization, undefined);
      return { status: 200, headers: {}, body: [Buffer.from('synthetic')] };
    },
  }, '');
  for (const path of ['/ready', '/v1/responses']) {
    const result = await apiRequest(port, path, { headers: { 'content-type': 'application/json' },
      method: path === '/ready' ? 'GET' : 'POST', body: path === '/ready' ? undefined : '{}',
    });
    assert.equal(result.response.status, 200);
  }
  assert.equal(calls, 2);
});

test('a configured server token still requires matching authorization', async context => {
  let calls = 0;
  const { port } = await instance(context, {
    ready() { calls += 1; return true; },
    forward() { throw new Error('unused'); },
  });
  assert.equal((await apiRequest(port, '/ready')).response.status, 401);
  assert.equal(calls, 0);
  assert.equal((await apiRequest(port, '/ready', { headers: authorizedHeaders() })).response.status, 200);
  assert.equal(calls, 1);
});

test('composition accepts synchronous readiness, responses and close', async context => {
  let closes = 0;
  const { app, port } = await instance(context, {
    ready() { return true; },
    forward(request) {
      assert.equal(request.headers.authorization, undefined);
      return { status: 200, headers: {}, body: [Buffer.from('synthetic')] };
    },
    close() { closes += 1; },
  });
  const ready = await apiRequest(port, '/ready', { headers: authorizedHeaders() });
  assert.equal(ready.response.status, 200);
  const response = await apiRequest(port, '/v1/responses', {
    method: 'POST', headers: authorizedHeaders(), body: '{}',
  });
  assert.equal(response.response.status, 200);
  assert.equal(response.body.toString(), 'synthetic');
  await app.close(); await app.close();
  assert.equal(closes, 1);
});
