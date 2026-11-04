import test from 'node:test';
import assert from 'node:assert/strict';
import { inspect } from 'node:util';
import { CompletedReplyError, MAX_COMPLETED_REPLY_BYTES,
  validateCompletedReply } from '../dist/compat/completed-reply.js';

const response = (overrides = {}) => ({
  id: 'resp_synthetic', object: 'response', model: 'synthetic-model', created_at: 1,
  status: 'completed', output: [], usage: { input_tokens: 1, output_tokens: 2, total_tokens: 3 },
  ...overrides,
});
const message = (overrides = {}) => ({
  id: 'msg_synthetic', type: 'message', role: 'assistant', model: 'synthetic-model',
  content: [{ type: 'text', text: 'Synthetic reply.' }], stop_reason: 'end_turn',
  stop_sequence: null, usage: { input_tokens: 1, output_tokens: 2 }, ...overrides,
});
const bytes = value => Buffer.from(JSON.stringify(value));

function accepts(protocol, value) {
  const input = bytes(value);
  const original = Buffer.from(input);
  assert.equal(validateCompletedReply(protocol, input), undefined);
  assert.deepEqual(input, original);
}

function rejects(protocol, input) {
  assert.throws(() => validateCompletedReply(protocol, input), error => {
    assert.ok(error instanceof CompletedReplyError);
    assert.equal(error.code, 'upstream_response_invalid');
    assert.equal(error.status, 502);
    assert.equal(error.retryable, false);
    assert.equal(inspect(error).includes('SYNTHETIC_SECRET'), false);
    return true;
  });
}

test('Responses text, refusal and output-limit results retain their native fields', () => {
  for (const content of [[{ type: 'output_text', text: 'Synthetic.', annotations: [] }],
    [{ type: 'refusal', refusal: 'Synthetic refusal.' }]]) {
    const output = [{ type: 'message', id: 'msg_synthetic', role: 'assistant', content }];
    accepts('responses', response({ output }));
    accepts('responses', response({ output, status: 'incomplete',
      incomplete_details: { reason: 'max_output_tokens' } }));
  }
});

test('tool-only Responses preserve arguments, custom text and optional item IDs', () => {
  accepts('responses', response({ output: [
    { type: 'function_call', call_id: 'call_synthetic', name: 'lookup',
      namespace: 'functions', arguments: '{ "query": "snow" }', status: 'completed' },
    { type: 'custom_tool_call', call_id: 'custom_synthetic', name: 'run_text', input: 'not JSON\n' },
    { type: 'reasoning', id: 'rs_synthetic', summary: [], encrypted_content: 'synthetic-history' },
  ] }));
});

test('extension fields and absent usage are not discarded or fabricated', () => {
  for (const usage of [null, undefined]) {
    accepts('responses', response({ usage, vendor_field: { nested: ['preserve'] },
      output: [{ type: 'custom_tool_call_output', id: 'out_synthetic', call_id: 'call_synthetic',
        output: [{ type: 'input_image', image_url: 'data:image/png;base64,AA==' }] }] }));
    accepts('messages', message({ usage, extra: { retained: true } }));
  }
});

test('Messages terminal reasons preserve refusal and limit outcomes', () => {
  for (const stop_reason of ['end_turn', 'max_tokens', 'stop_sequence', 'pause_turn',
    'refusal', 'model_context_window_exceeded']) accepts('messages', message({ stop_reason,
      stop_sequence: stop_reason === 'stop_sequence' ? 'synthetic-stop' : null }));
});

test('Messages stop reasons must match the returned tool or stop sequence', () => {
  rejects('messages', bytes(message({ stop_reason: 'tool_use' })));
  rejects('messages', bytes(message({ stop_reason: 'stop_sequence' })));
  rejects('messages', bytes(message({ stop_reason: 'stop_sequence', stop_sequence: undefined })));
});

test('Messages tool-only output and signed history stay intact', () => {
  accepts('messages', message({ stop_reason: 'tool_use', content: [
    { type: 'thinking', thinking: 'Synthetic thought.', signature: 'synthetic-signature' },
    { type: 'redacted_thinking', data: 'synthetic-encrypted-content' },
    { type: 'tool_use', id: 'tool_synthetic', name: 'lookup', input: { query: 'snow' } },
  ] }));
});

for (const status of ['in_progress', 'queued', 'failed', 'cancelled', null, ['completed']]) {
  test(`Responses rejects a nonterminal or invalid status: ${JSON.stringify(status)}`, () => {
    rejects('responses', bytes(response({ status })));
  });
}

for (const stop_reason of [null, undefined, 'in_progress', ['end_turn']]) {
  test(`Messages rejects a missing or invalid stop: ${JSON.stringify(stop_reason)}`, () => {
    rejects('messages', bytes(message({ stop_reason })));
  });
}

test('Responses rejects missing completion fields and partial tool arguments', () => {
  const tool = { type: 'function_call', call_id: 'call_synthetic', name: 'lookup', arguments: '{}' };
  for (const value of [
    response({ id: '' }), response({ model: '' }), response({ object: 'error' }),
    response({ created_at: -1 }), response({ output: {} }), response({ output: [null] }),
    response({ error: { message: 'SYNTHETIC_SECRET' } }),
    response({ status: 'incomplete' }),
    response({ output: [{ ...tool, arguments: '{"SYNTHETIC_SECRET":' }] }),
    response({ output: [{ ...tool, call_id: undefined }] }),
    response({ output: [{ ...tool, status: 'in_progress' }] }),
    response({ output: [{ type: 'message', id: 'msg', role: 'user', content: [] }] }),
  ]) rejects('responses', bytes(value));
});

test('Messages rejects malformed text, signed history and tool calls', () => {
  for (const content of [
    [{ type: 'text', text: 123 }], [{ type: 'thinking', thinking: 'text' }],
    [{ type: 'redacted_thinking', data: '' }],
    [{ type: 'tool_use', id: 'tool', name: 'lookup', input: 'SYNTHETIC_SECRET' }],
    [{ type: 'tool_use', name: 'lookup', input: {} }], [null],
  ]) rejects('messages', bytes(message({ content })));
  rejects('messages', bytes(message({ type: 'error', error: { message: 'SYNTHETIC_SECRET' } })));
  rejects('messages', bytes(message({ role: 'user' })));
});

test('supplied usage must contain nonnegative integer token counts', () => {
  for (const count of [-1, 0.5, '1', null]) {
    const usage = { input_tokens: count, output_tokens: 1 };
    rejects('responses', bytes(response({ usage })));
    rejects('messages', bytes(message({ usage })));
  }
});

test('invalid bytes and protocol errors have fixed non-retryable errors', () => {
  for (const input of [Buffer.alloc(0), Buffer.from([0xff]),
    Buffer.from('{"SYNTHETIC_SECRET":'), bytes(null), bytes([]), 'SYNTHETIC_SECRET']) {
    rejects('responses', input);
  }
  rejects('unknown', bytes(response()));
  rejects('messages', bytes(response()));
  rejects('responses', bytes(message()));
});

test('the byte limit accepts its boundary and rejects overflow before parsing', () => {
  const input = bytes(response());
  const padded = Buffer.alloc(MAX_COMPLETED_REPLY_BYTES, 32);
  input.copy(padded);
  validateCompletedReply('responses', padded);
  rejects('responses', Buffer.alloc(MAX_COMPLETED_REPLY_BYTES + 1, 32));
});
