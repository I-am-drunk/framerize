import assert from 'node:assert/strict';
import test from 'node:test';
import { setTimeout as delay } from 'node:timers/promises';
import { createInstance } from '../src/instance.js';
import { instanceOptions } from '../src/instance-options.js';
import { authorization, backend, exchange, token } from './http-fixtures.js';

test('inference has its own validated deadline, separate from readiness', () => {
  const config = instanceOptions({ token, backend: backend() });
  assert.equal(config.inferenceTimeoutMs, 300_000);
  assert.equal(config.headersTimeoutMs, 15_000);
  for (const inferenceTimeoutMs of [0, -1, 1.5, NaN, Infinity, 2_147_483_648]) {
    assert.throws(() => instanceOptions({ token, backend: backend(), inferenceTimeoutMs }),
      { message: 'Invalid instance configuration' });
  }
});

test('completed JSON can outlive readiness and body idle limits', { timeout: 3000 }, async () => {
  const instance = createInstance({ token, headersTimeoutMs: 40, idleTimeoutMs: 40,
    inferenceTimeoutMs: 1000, backend: backend({ async forward({ signal }) {
      await delay(160, undefined, { signal });
      return { status: 200, headers: {}, body: [Buffer.from('{"done":true}')] };
    } }) });
  try {
    const { port } = await instance.listen();
    const response = await exchange(port, '/v1/responses', { method: 'POST', headers: authorization },
      Buffer.from('{"stream":false}'));
    assert.equal(response.status, 200);
    assert.equal(response.body.toString(), '{"done":true}');
  } finally { await instance.close(); }
});
