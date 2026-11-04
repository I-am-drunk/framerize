import assert from 'node:assert/strict';
import test from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';
import { completedRequestHeaders, requestHeaders, selectedHeaders } from '../src/http-policy.js';
import { createInstance } from '../src/instance.js';
import { authorization, backend, exchange, token } from './http-fixtures.js';

test('completed routes accept Anthropic protocol headers in the forwarded set', () => {
  const headers = selectedHeaders({
    accept: 'application/json', 'content-type': 'application/json',
    'anthropic-version': '2026-10-21', 'anthropic-beta': 'messages-2024-01-01',
    'x-custom': 'drop-me',
  }, completedRequestHeaders);
  assert.deepEqual(headers, {
    accept: 'application/json', 'content-type': 'application/json',
    'anthropic-version': '2026-10-21', 'anthropic-beta': 'messages-2024-01-01',
  });
  assert.deepEqual(selectedHeaders(headers, requestHeaders), {
    accept: 'application/json', 'content-type': 'application/json',
  });
});

for (const path of ['/openai/v1/responses', '/anthropic/v1/messages'] as const) {
  test(`${path} forwards completed headers to the backend`, async () => {
    const forwarded: Record<string, string>[] = [];
    const instance = createInstance({ token, backend: backend({
      async forward(request) {
        forwarded.push({ ...request.headers });
        return { status: 200, headers: { 'content-type': 'application/json' },
          body: [Buffer.from('{"ok":true}')] };
      },
    }) });
    try {
      const { port } = await instance.listen();
      const result = await exchange(port, path, {
        method: 'POST', headers: {
          ...authorization, accept: 'application/json', 'content-type': 'application/json',
          'anthropic-version': '2026-10-21', 'anthropic-beta': 'messages-2024-01-01',
        },
      }, Buffer.from('{}'));
      assert.equal(result.status, 200);
      assert.deepEqual(forwarded.at(-1), {
        accept: 'application/json', 'content-type': 'application/json',
        'anthropic-version': '2026-10-21', 'anthropic-beta': 'messages-2024-01-01',
      });
    } finally { await instance.close(); }
  });
}

test('Anthropic completed routes return Anthropic-shaped upstream failures', async () => {
  const instance = createInstance({ token, backend: backend({
    async forward() {
      throw Object.assign(new Error('hidden'), { code: 'upstream_response_invalid', status: 502, retryable: false });
    },
  }) });
  try {
    const { port } = await instance.listen();
    const failure = await exchange(port, '/anthropic/v1/messages', { method: 'POST', headers: authorization });
    assert.equal(failure.status, 502);
    assert.equal(failure.headers['x-should-retry'], 'false');
    const body = JSON.parse(failure.body.toString());
    assert.equal(body.type, 'error');
    assert.equal(body.error.code, 'upstream_response_invalid');
    assert.equal(body.error.type, 'api_error');
  } finally { await instance.close(); }
});

test('completed routes honor the inference deadline for slow upstream JSON', { timeout: 3000 }, async () => {
  const instance = createInstance({ token, inferenceTimeoutMs: 120, idleTimeoutMs: 40, headersTimeoutMs: 40,
    backend: backend({ async forward({ signal }) {
      await delay(200, undefined, { signal });
      return { status: 200, headers: {}, body: [Buffer.from('{"done":true}')] };
    } }) });
  try {
    const { port } = await instance.listen();
    const failure = await exchange(port, '/openai/v1/responses', { method: 'POST', headers: authorization },
      Buffer.from('{}'));
    assert.equal(failure.status, 504);
    assert.equal(failure.headers['x-should-retry'], 'false');
    assert.equal(JSON.parse(failure.body.toString()).error.code, 'headers_timeout');
  } finally { await instance.close(); }
});
