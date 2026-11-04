import assert from 'node:assert/strict';
import test from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';
import { createInstance } from '../src/instance.js';
import { HttpError } from '../src/http-errors.js';
import { authorization, backend, exchange, token } from './http-fixtures.js';

for (const code of ['headers_timeout', 'idle_timeout', 'upstream_error'] as const) {
  test(`locally generated ${code} preserves the retry boundary`, async () => {
    const instance = createInstance({ token, backend: backend({
      async forward() { throw new HttpError(code); },
    }) });
    try {
      const { port } = await instance.listen();
      const result = await exchange(port, '/v1/responses',
        { method: 'POST', headers: authorization }, Buffer.from('{}'));
      assert.equal(result.status, code === 'upstream_error' ? 502 : 504);
      assert.equal(result.headers['x-should-retry'], code === 'upstream_error' ? undefined : 'false');
    } finally { await instance.close(); }
  });
}

test('an upstream 504 without a retry instruction remains unchanged', async () => {
  const instance = createInstance({ token, backend: backend({ async forward() {
    return { status: 504, headers: {}, body: [Buffer.from('{}')] };
  } }) });
  try {
    const { port } = await instance.listen();
    const result = await exchange(port, '/v1/responses',
      { method: 'POST', headers: authorization }, Buffer.from('{}'));
    assert.equal(result.status, 504);
    assert.equal(result.headers['x-should-retry'], undefined);
  } finally { await instance.close(); }
});

test('an upstream no-retry instruction is preserved on forwarded responses', async () => {
  const instance = createInstance({ token, backend: backend({ async forward() {
    return { status: 503, headers: { 'x-should-retry': 'false' }, body: [Buffer.from('{}')] };
  } }) });
  try {
    const { port } = await instance.listen();
    const result = await exchange(port, '/v1/responses',
      { method: 'POST', headers: authorization }, Buffer.from('{}'));
    assert.equal(result.status, 503);
    assert.equal(result.headers['x-should-retry'], 'false');
  } finally { await instance.close(); }
});

for (const path of ['/v1/responses', '/v1/chat/completions']) {
  test(`${path} deadline discourages retries and aborts inference`, async () => {
    let calls = 0;
    let aborted = false;
    const instance = createInstance({ token, inferenceTimeoutMs: 30,
      backend: backend({ async forward({ signal }) {
        calls++;
        try { await delay(1000, undefined, { signal }); }
        finally { aborted = signal.aborted; }
        return { status: 200, headers: {}, body: [] };
      } }) });
    try {
      const { port } = await instance.listen();
      const result = await exchange(port, path, { method: 'POST', headers: authorization }, Buffer.from('{}'));
      assert.equal(result.status, 504);
      assert.equal(result.headers['x-should-retry'], 'false');
      assert.equal(calls, 1);
      assert.equal(aborted, true);
    } finally { await instance.close(); }
  });
}
