import assert from 'node:assert/strict';
import test from 'node:test';
import { responseHeaders, selectedHeaders } from '../src/http-policy.js';
import { createInstance } from '../src/instance.js';
import { authorization, backend, exchange, token } from './http-fixtures.js';

const cases: readonly { name: string; value: unknown; expected?: string }[] = [
  { name: 'explicit no-retry', value: 'false', expected: 'false' },
  { name: 'retry instruction', value: 'true' },
  { name: 'unknown value', value: 'unknown' },
  { name: 'joined duplicates', value: 'false, false' },
  { name: 'array duplicates', value: ['false', 'false'] },
  { name: 'non-string value', value: false },
  { name: 'missing value', value: undefined },
];

for (const scenario of cases) {
  test(`response retry header: ${scenario.name}`, () => {
    const headers = selectedHeaders({ 'X-Should-Retry': scenario.value }, responseHeaders);
    assert.deepEqual(headers, scenario.expected ? { 'x-should-retry': scenario.expected } : {});
  });
}

test('Connection nomination removes an otherwise valid no-retry header', () => {
  assert.deepEqual(selectedHeaders({
    'x-should-retry': 'false', connection: 'keep-alive, X-Should-Retry',
  }, responseHeaders), {});
});

test('duplicate Connection field values are all honored when nominating retry', () => {
  assert.deepEqual(selectedHeaders({
    'x-should-retry': 'false',
    connection: ['keep-alive', 'x-should-retry'],
  }, responseHeaders), {});
});

test('the listener drops retry guidance when Connection lists nominate it in pieces', async () => {
  const headers: Record<string, string | string[]> = {
    'x-should-retry': 'false', connection: ['keep-alive', 'x-should-retry'],
  };
  const instance = createInstance({ token, backend: backend({
    async forward() {
      return { status: 502, headers: headers as Record<string, string>, body: [Buffer.from('{}')] };
    },
  }) });
  try {
    const { port } = await instance.listen();
    const result = await exchange(port, '/v1/responses', {
      method: 'POST', headers: authorization,
    }, Buffer.from('{}'));
    assert.equal(result.status, 502);
    assert.equal(result.headers['x-should-retry'], undefined);
  } finally { await instance.close(); }
});

for (const values of [['false', 'false'], ['true', 'false'], ['false', 'true']]) {
  test(`case-insensitive retry duplicates are rejected: ${values.join(', ')}`, () => {
    assert.deepEqual(selectedHeaders({
      'X-Should-Retry': values[0], 'x-should-retry': values[1],
    }, responseHeaders), {});
  });
}

test('the listener drops differently cased retry duplicates from a custom backend', async () => {
  const instance = createInstance({ token, backend: backend({
    async forward() {
      return { status: 502, headers: { 'X-Should-Retry': 'true', 'x-should-retry': 'false' },
        body: [Buffer.from('{}')] };
    },
  }) });
  try {
    const { port } = await instance.listen();
    const result = await exchange(port, '/v1/responses', {
      method: 'POST', headers: authorization,
    }, Buffer.from('{}'));
    assert.equal(result.status, 502);
    assert.equal(result.headers['x-should-retry'], undefined);
  } finally { await instance.close(); }
});

for (const connection of [
  { Connection: 'X-Should-Retry', connection: 'keep-alive' },
  { connection: 'x-should-retry', Connection: 'keep-alive' },
]) {
  const label = Object.keys(connection).join(', ');
  const headers = { 'x-should-retry': 'false', ...connection };
  test(`all Connection nominations are honored: ${label}`, () => {
    assert.deepEqual(selectedHeaders(headers, responseHeaders), {});
  });

  test(`the listener honors all Connection nominations: ${label}`, async () => {
    const instance = createInstance({ token, backend: backend({
      async forward() {
        return { status: 502, headers, body: [Buffer.from('{}')] };
      },
    }) });
    try {
      const { port } = await instance.listen();
      const result = await exchange(port, '/v1/responses', {
        method: 'POST', headers: authorization,
      }, Buffer.from('{}'));
      assert.equal(result.status, 502);
      assert.equal(result.headers['x-should-retry'], undefined);
    } finally { await instance.close(); }
  });
}
