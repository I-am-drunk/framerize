import assert from 'node:assert/strict';
import test from 'node:test';
import { decodeFramerRequest, encodeFramerRequest } from '../dist/framer-request.js';

function encoded(request, path) {
  const original = JSON.stringify(request);
  const descriptor = Object.getOwnPropertyDescriptor(String.prototype, Symbol.iterator);
  let visited = 0;
  let bytes;
  try {
    Object.defineProperty(String.prototype, Symbol.iterator, { ...descriptor,
      value: function* () {
        for (const character of descriptor.value.call(this)) {
          visited += 1;
          yield character;
        }
      },
    });
    bytes = encodeFramerRequest(decodeFramerRequest(path, Buffer.from(original)),
      'synthetic-project', 'synthetic-token');
  } finally { Object.defineProperty(String.prototype, Symbol.iterator, descriptor); }
  const envelope = JSON.parse(bytes.toString());
  assert.equal(JSON.stringify(envelope.request), original);
  return { envelope, visited };
}

for (const shape of ['input', 'content']) {
  test(`prompt prefix work stays bounded for long ${shape}`, context => {
    const prefix = `${'a'.repeat(298)}\n🦊`;
    const text = `${prefix}${'z'.repeat(200_000)}`;
    const request = shape === 'input' ? { model: 'synthetic', input: text }
      : { model: 'synthetic', messages: [
        { role: 'user', content: [{ text }, { input_text: 'tail' }] },
        { role: 'assistant', content: 'Do not use the last assistant message' },
      ] };
    const path = shape === 'input' ? '/v1/responses' : '/v1/chat/completions';
    const result = encoded(request, path);
    assert.equal(result.envelope.userPrompt, prefix);
    context.diagnostic(JSON.stringify({ shape, visited: result.visited,
      prefixPreserved: true, requestPreserved: true, runtime: process.version }));
    assert.ok(result.visited <= 600, 'Prefix extraction must not expand the entire request string');
  });
}

test('short prompt metadata retains control characters and lone surrogates', () => {
  const text = 'line\r\nNUL\0lone\uD800end';
  const { envelope, visited } = encoded({ model: 'synthetic', input: text }, '/v1/responses');
  assert.equal(envelope.userPrompt, text);
  assert.ok(visited <= 600);
});
