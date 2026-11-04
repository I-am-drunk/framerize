import assert from 'node:assert/strict';
import test from 'node:test';
import { createInstance } from '../src/instance.js';
import { authorization, backend, exchange, token } from './http-fixtures.js';

const completedRoutes = ['/openai/v1/responses', '/anthropic/v1/messages'] as const;

test('completed custom routes return buffered upstream bodies without streaming', async () => {
  for (const route of completedRoutes) {
    const payload = Buffer.from(`{"route":"${route}"}`);
    const instance = createInstance({ token, backend: backend({
      async forward(request) {
        assert.equal(request.path, route);
        return { status: 200, headers: { 'content-type': 'application/json' }, body: [payload] };
      },
    }) });
    try {
      const { port } = await instance.listen();
      const response = await exchange(port, route, { method: 'POST', headers: authorization });
      assert.equal(response.status, 200);
      assert.deepEqual(response.body, payload);
    } finally { await instance.close(); }
  }
});

test('completed Anthropic routes forward protocol headers and drop private ones', async () => {
  let forwarded: Record<string, string> | undefined;
  const instance = createInstance({ token, backend: backend({
    async forward(request) {
      forwarded = { ...request.headers };
      return { status: 200, headers: {}, body: [Buffer.from('{}')] };
    },
  }) });
  try {
    const { port } = await instance.listen();
    await exchange(port, '/anthropic/v1/messages', { method: 'POST', headers: {
      ...authorization, 'content-type': 'application/json', 'anthropic-version': '2026-10-21',
      'anthropic-beta': 'synthetic-beta', cookie: 'synthetic-private', 'x-api-key': 'hidden' } });
    assert.equal(forwarded?.['anthropic-version'], '2026-10-21');
    assert.equal(forwarded?.['anthropic-beta'], 'synthetic-beta');
    assert.equal(forwarded?.cookie, undefined);
    assert.equal(forwarded?.['x-api-key'], undefined);
  } finally { await instance.close(); }
});

test('auth failures use the native error envelope for each completed route', async () => {
  const instance = createInstance({ token, backend: backend() });
  try {
    const { port } = await instance.listen();
    const openai = JSON.parse((await exchange(port, '/openai/v1/responses',
      { method: 'POST' })).body.toString());
    assert.equal(openai.error.type, 'authentication_error');
    assert.equal(openai.type, undefined);

    const anthropic = JSON.parse((await exchange(port, '/anthropic/v1/messages',
      { method: 'POST' })).body.toString());
    assert.equal(anthropic.type, 'error');
    assert.equal(anthropic.error.type, 'authentication_error');
  } finally { await instance.close(); }
});

test('completed route failures honor upstream status and disable replay', async () => {
  const failure = Object.assign(new Error('Remote completed request failed'), {
    code: 'upstream_response_invalid', status: 502, retryable: false,
  });
  const instance = createInstance({ token, backend: backend({ async forward() { throw failure; } }) });
  try {
    const { port } = await instance.listen();
    const response = await exchange(port, '/openai/v1/responses', { method: 'POST', headers: authorization });
    assert.equal(response.status, 502);
    assert.equal(response.headers['x-should-retry'], 'false');
    assert.equal(JSON.parse(response.body.toString()).error.code, 'upstream_response_invalid');
  } finally { await instance.close(); }
});
