import test from 'node:test';
import assert from 'node:assert/strict';
import { prepareFramer } from '../src/framer.js';

const account = { id: 'fixture', provider: 'framer', projectId: 'project',
  upstream: 'https://api.framer.test', authorizationEnv: 'FIXTURE_JWT', castleEnv: 'FIXTURE_CASTLE' };

test('Framer requests use the native Responses envelope', async () => {
  const calls = [];
  const result = await prepareFramer({ account, path: '/v1/responses',
    body: Buffer.from(JSON.stringify({ model: 'gpt-test', input: 'hello', stream: true })),
    fetchImpl: async (...args) => { calls.push(args); }, env: {
      FIXTURE_JWT: 'fixture-jwt', FIXTURE_CASTLE: 'fixture-castle',
    } });
  const envelope = JSON.parse(result.body);
  assert.equal(result.url, 'https://api.framer.test/ai/v3/openai/responses/');
  assert.equal(result.authorization, 'Bearer fixture-jwt');
  assert.equal(envelope.request.input, 'hello');
  assert.equal(envelope.projectId, 'project');
  assert.equal(envelope.castleToken, 'fixture-castle');
  assert.equal(calls.length, 0);
});

test('Framer chat uses its native Chat route', async () => {
  const result = await prepareFramer({ account, path: '/v1/chat/completions',
    body: Buffer.from('{"model":"gpt-test","messages":[]}'), fetchImpl: async () => {},
    env: { FIXTURE_JWT: 'fixture-jwt', FIXTURE_CASTLE: 'fixture-castle' } });
  assert.equal(result.url, 'https://api.framer.test/ai/v3/chat/');
});

test('missing credentials fail before an inference request', async () => {
  await assert.rejects(prepareFramer({ account, path: '/v1/responses', body: Buffer.from('{}'),
    fetchImpl: async () => { throw new Error('unexpected fetch'); }, env: {} }), /no Castle credential/);
});

test('JWT minting is cached per account', async () => {
  let calls = 0;
  const scoped = { ...account, id: 'mint-fixture', authorizationEnv: undefined, sessionEnv: 'TEST_SESSION' };
  const env = { TEST_SESSION: 'session', TEST_CASTLE: 'castle', FIXTURE_CASTLE: 'castle' };
  const fetchImpl = async () => {
    calls += 1;
    return new Response(JSON.stringify({ accessToken: 'minted', expiresAt: new Date(Date.now() + 600000).toISOString() }));
  };
  for (let index = 0; index < 2; index += 1) {
    await prepareFramer({ account: scoped, path: '/v1/responses', body: Buffer.from('{}'), fetchImpl, env });
  }
  assert.equal(calls, 1);
});

test('concurrent requests share one JWT refresh', async () => {
  let calls = 0;
  const scoped = { ...account, id: 'concurrent-fixture', authorizationEnv: undefined, sessionEnv: 'TEST_SESSION' };
  const fetchImpl = async () => {
    calls += 1;
    await new Promise((resolve) => setTimeout(resolve, 10));
    return new Response(JSON.stringify({ accessToken: 'minted', expiresAt: new Date(Date.now() + 600000).toISOString() }));
  };
  await Promise.all(Array.from({ length: 4 }, () => prepareFramer({ account: scoped,
    path: '/v1/responses', body: Buffer.from('{}'), fetchImpl,
    env: { TEST_SESSION: 'session', FIXTURE_CASTLE: 'castle' } })));
  assert.equal(calls, 1);
});

test('Framer authentication failures preserve their status and retry cleanly', async () => {
  const scoped = { ...account, id: 'auth-failure-fixture', authorizationEnv: undefined, sessionEnv: 'TEST_SESSION' };
  const env = { TEST_SESSION: 'session', FIXTURE_CASTLE: 'castle' };
  const missing = prepareFramer({ account: scoped, path: '/v1/responses', body: Buffer.from('{}'),
    fetchImpl: async () => { throw new Error('unexpected fetch'); }, env: {} });
  await assert.rejects(missing, (error) => error.status === 503);
  let calls = 0;
  const fetchImpl = async () => {
    calls += 1;
    return new Response('{}', { status: calls === 1 ? 401 : 503 });
  };
  for (const status of [401, 502]) {
    await assert.rejects(prepareFramer({ account: scoped, path: '/v1/responses',
      body: Buffer.from('{}'), fetchImpl, env }), (error) => error.status === status);
  }
  const invalid = prepareFramer({ account: scoped, path: '/v1/responses', body: Buffer.from('{}'),
    fetchImpl: async () => new Response('{}'), env });
  await assert.rejects(invalid, /invalid Framer token response/);
});

