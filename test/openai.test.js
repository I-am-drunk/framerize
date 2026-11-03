test('invalid chat inputs return validation errors', () => {
  assert.throws(() => validateChat({ messages: [] }), /messages is required/);
  assert.throws(() => validateChat({ messages: [{ role: 'user', content: 'x' }], n: 2 }), /n must be 1/);
  assert.throws(() => validateChat({ messages: [{ role: 'user', content: 'x' }],
    response_format: { type: 'json_schema', json_schema: null } }), /must be an object/);
  assert.throws(() => validateChat({ messages: [{ role: 'unknown', content: 'x' }] }), /unsupported message role/);
});

test('upstream response failures retain a sanitized error', () => {
  const failure = responseEventError({ type: 'response.failed',
    response: { error: { message: 'upstream rejected request' } } });
  assert.equal(failure.status, 502);
  assert.match(failure.message, /rejected/);
  assert.equal(responseEventError({ type: 'response.completed' }), null);
});

import test from 'node:test';
import assert from 'node:assert/strict';
import { validateChat, responseEventError } from '../src/openai.js';

