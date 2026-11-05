import assert from 'node:assert/strict';
import test from 'node:test';
import { createInstance } from '../src/instance.js';
import type { InstanceOptions } from '../src/instance-options.js';
import { authorization, backend, exchange, token } from './http-fixtures.js';

test('required authentication stays the default, including with a null token', async () => {
  for (const options of [{ token }, { token: null }, { token, authentication: 'required' }] as const) {
    const instance = createInstance({ ...options, backend: backend() });
    try {
      const { port } = await instance.listen();
      assert.equal((await exchange(port, '/ready')).status, 401);
      assert.equal((await exchange(port, '/v1/responses', { method: 'POST' })).status, 401);
      const expected = options.token === null ? 401 : 200;
      assert.equal((await exchange(port, '/ready', { headers: authorization })).status, expected);
    } finally { await instance.close(); }
  }
});

test('authentication none serves protected routes only when explicitly selected', async () => {
  const instance = createInstance({ token: null, authentication: 'none', backend: backend() });
  try {
    const { port } = await instance.listen();
    assert.equal((await exchange(port, '/ready')).status, 200);
    assert.equal((await exchange(port, '/v1/responses', { method: 'POST' })).status, 200);
    assert.equal((await exchange(port, '/ready', { headers: authorization })).status, 200);
  } finally { await instance.close(); }
});

test('authentication none rejects a configured token and unknown modes', () => {
  for (const options of [{ token, authentication: 'none' }, { token: null, authentication: 'off' }]) {
    assert.throws(() => createInstance({ ...options, backend: backend() } as unknown as InstanceOptions),
      { message: 'Invalid instance configuration' });
  }
});
