import assert from 'node:assert/strict';
import test from 'node:test';
import { createInstance } from '../src/instance.js';
import { authorization, backend, exchange, token } from './http-fixtures.js';

test('rejected synchronous bodies are returned without a read', async () => {
  let returned = 0;
  const instance = createInstance({ token, backend: backend({ async forward() {
    return { status: 199, headers: {}, body: { [Symbol.iterator]() { return {
      next() { throw new Error('Discarded body must not be read'); },
      return() { returned += 1; return { done: true as const, value: undefined }; },
    }; } } };
  } }) });
  try {
    const { port } = await instance.listen();
    assert.equal((await exchange(port, '/v1/responses', { method: 'POST', headers: authorization })).status, 502);
    assert.equal(returned, 1);
  } finally { await instance.close(); }
});

test('synchronous response buffers preserve the server embedding contract', async () => {
  const instance = createInstance({ token, backend: backend({
    async forward() { return { status: 200, headers: {}, body: [Buffer.from('one'), Buffer.from('two')] }; },
  }) });
  try {
    const { port } = await instance.listen();
    const result = await exchange(port, '/v1/responses', { method: 'POST', headers: authorization });
    assert.equal(result.status, 200);
    assert.equal(result.body.toString(), 'onetwo');
  } finally { await instance.close(); }
});

test('an explicitly absent token locks every protected route', async () => {
  let calls = 0;
  const instance = createInstance({ token: null, backend: backend({
    async ready() { calls += 1; return true; },
    async forward() { calls += 1; throw new Error('Must not run'); },
  }) });
  try {
    const { port } = await instance.listen();
    assert.equal((await exchange(port, '/health')).status, 200);
    for (const path of ['/ready', '/v1/responses', '/v1/chat/completions']) {
      for (const headers of [{}, authorization, { authorization: 'Bearer null' }]) {
        const result = await exchange(port, path, { headers, method: path === '/ready' ? 'GET' : 'POST' });
        assert.equal(result.status, 401);
      }
    }
    assert.equal(calls, 0);
  } finally { await instance.close(); }
});
