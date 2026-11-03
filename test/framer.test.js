import test from 'node:test';
import assert from 'node:assert/strict';
import { prepareFramer } from '../src/framer.js';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

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
  assert.match(envelope.sessionId, /^[A-Za-z0-9]{9}$/);
  assert.match(envelope.turnId, /^[A-Za-z0-9]{9}$/);
  assert.notEqual(envelope.sessionId, envelope.turnId);
  assert.equal(calls.length, 0);
});

test('Framer chat uses its native Chat route', async () => {
  const result = await prepareFramer({ account, path: '/v1/chat/completions',
    body: Buffer.from('{"model":"gpt-test","stream":true,"messages":[]}'), fetchImpl: async () => {},
    env: { FIXTURE_JWT: 'fixture-jwt', FIXTURE_CASTLE: 'fixture-castle' } });
  assert.equal(result.url, 'https://api.framer.test/ai/v3/chat/');
  const envelope = JSON.parse(result.body);
  assert.deepEqual(envelope.request, {
    model: 'gpt-test', stream: true, messages: [],
  });
  assert.equal(envelope.projectId, 'project');
  assert.equal(envelope.featureCategory, 'agents');
  assert.equal(envelope.featureName, 'agents');
  assert.equal(envelope.model, 'gpt-test');
  assert.equal(envelope.step, 'message');
  assert.equal(envelope.stepIndex, 0);
  assert.equal(envelope.userPrompt, '');
  assert.equal(envelope.castleToken, 'fixture-castle');
  assert.match(envelope.sessionId, /^[A-Za-z0-9]{9}$/);
  assert.match(envelope.turnId, /^[A-Za-z0-9]{9}$/);
});

test('Framer chat preserves tools, tool results, and JSON schemas', async () => {
  const request = { model: 'anthropic/claude-opus-5.5', stream: true,
    messages: [
      { role: 'user', content: 'What is the weather?' },
      { role: 'assistant', content: null, tool_calls: [{ id: 'call1', type: 'function',
        function: { name: 'weather', arguments: '{"city":"Paris"}' } }] },
      { role: 'tool', tool_call_id: 'call1', content: 'sunny' },
    ],
    tools: [{ type: 'function', function: { name: 'weather',
      parameters: { type: 'object', properties: { city: { type: 'string' } } } } }],
    tool_choice: { type: 'function', function: { name: 'weather' } },
    response_format: { type: 'json_schema', json_schema: { name: 'answer',
      schema: { type: 'object' } } } };
  const result = await prepareFramer({ account, path: '/v1/chat/completions',
    body: Buffer.from(JSON.stringify(request)), fetchImpl: async () => {},
    env: { FIXTURE_JWT: 'fixture-jwt', FIXTURE_CASTLE: 'fixture-castle' } });
  assert.deepEqual(JSON.parse(result.body).request, request);
});

test('Framer reads credentials from private files', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-framer-'));
  const sessionFile = path.join(directory, 'session');
  const castleFile = path.join(directory, 'castle');
  await writeFile(sessionFile, 'session=file-secret');
  await writeFile(castleFile, 'file-castle');
  const scoped = { ...account, sessionFile, castleFile, authorizationEnv: undefined, castleEnv: undefined };
  const result = await prepareFramer({ account: scoped, path: '/v1/responses',
    body: Buffer.from('{}'), fetchImpl: async () => new Response(JSON.stringify({
      accessToken: 'minted', expiresAt: new Date(Date.now() + 600000).toISOString(),
    })) });
  assert.equal(JSON.parse(result.body).castleToken, 'file-castle');
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

