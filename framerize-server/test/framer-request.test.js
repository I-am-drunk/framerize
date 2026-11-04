import test from 'node:test';
import assert from 'node:assert/strict';
import { decodeFramerRequest, encodeFramerRequest, FRAMER_ROUTES } from '../dist/framer-request.js';

for (const path of ['/openai/v1/responses', '/anthropic/v1/messages']) {
  for (const stream of [undefined, false, true]) {
    test(`${path} separates client stream=${stream} from provider mode`, () => {
      const request = { model: 'fixture', input: 'hello', messages: [], max_tokens: 10,
        stream, temperature: 0.2, tools: [{ type: 'custom', name: 'fixture' }] };
      const decoded = decodeFramerRequest(path, Buffer.from(JSON.stringify(request)));
      assert.equal(decoded.clientStream, stream === true);
      assert.deepEqual(decoded.request, { ...request, stream: false });
      const encoded = JSON.parse(encodeFramerRequest(decoded, 'project', 'synthetic'));
      assert.deepEqual(encoded.request, decoded.request);
    });
  }
}

for (const [path, request] of [
  ['/v1/responses', { model: 'example', input: 'hello', stream: false, previous_response_id: 'response-1' }],
  ['/v1/chat/completions', { model: 'example/model', messages: [], stream: false }],
  ['/v1/chat/completions', { model: 'example/model', messages: [], stream: true, stream_options: { include_usage: true } }],
]) {
  test(`${path} preserves the native request and streaming choice`, () => {
    const decoded = decodeFramerRequest(path, Buffer.from(JSON.stringify(request)));
    const result = JSON.parse(encodeFramerRequest(decoded, 'project-1', 'synthetic-castle'));
    assert.deepEqual(result.request, request);
    assert.equal(result.featureCategory, 'agents');
    assert.equal(result.featureName, 'agents');
    assert.equal(result.model, request.model);
    assert.equal(result.castleToken, 'synthetic-castle');
    assert.equal(result.projectId, 'project-1');
    assert.equal(result.step, 'message');
    assert.equal(result.stepIndex, 0);
    assert.match(result.sessionId, /^[0-9a-f-]{36}$/);
    assert.match(result.turnId, /^[0-9a-f-]{36}$/);
    assert.notEqual(result.sessionId, result.turnId);
    assert.ok(Object.isFrozen(FRAMER_ROUTES));
  });
}

for (const value of ['{', 'null', '[]', '{}', '{"model":" "}', '{"model":1}',
  '{"model":"example","stream":"true"}', '{"model":"example","messages":{}}']) {
  test('invalid chat input has a fixed public error', () => {
    assert.throws(() => decodeFramerRequest('/v1/chat/completions', Buffer.from(value)),
      { code: 'invalid_request_body', status: 400, message: 'Invalid inference request' });
  });
}

test('prompt metadata uses the last user text and clips Unicode without changing input', () => {
  const text = '🦊'.repeat(400);
  const input = { model: 'example', messages: [{ role: 'user', content: 'old' },
    { role: 'user', content: [{ input_text: text }] }, { role: 'assistant', content: 'later' }] };
  const decoded = decodeFramerRequest('/v1/chat/completions', Buffer.from(JSON.stringify(input)));
  const envelope = JSON.parse(encodeFramerRequest(decoded, 'project', 'synthetic'));
  assert.equal(envelope.userPrompt, '🦊'.repeat(300));
  assert.deepEqual(envelope.request, input);
  assert.throws(() => decodeFramerRequest('/v1/responses', Buffer.from([0xff])), { code: 'invalid_request_body' });
  assert.throws(() => decodeFramerRequest('/unknown', Buffer.from(JSON.stringify(input))), { code: 'invalid_request_body' });
});
