import test from 'node:test';
import assert from 'node:assert/strict';
import { chatToResponses, responseToChat, responsesToChatStream } from '../src/openai.js';

test('chat requests preserve messages, tools, results and schemas', () => {
  const converted = chatToResponses({ model: 'gpt-test', messages: [
    { role: 'system', content: 'system' }, { role: 'user', content: 'question' },
    { role: 'assistant', content: null, tool_calls: [{ id: 'call1', type: 'function',
      function: { name: 'lookup', arguments: '{"q":"x"}' } }] },
    { role: 'tool', tool_call_id: 'call1', content: 'answer' },
  ], tools: [{ type: 'function', function: { name: 'lookup', parameters: {}, strict: false } }],
    tool_choice: { type: 'function', function: { name: 'lookup' } },
    response_format: { type: 'json_schema', json_schema: { name: 'answer', schema: {} } },
  });
  assert.equal(converted.input.length, 4);
  assert.equal(converted.input[2].type, 'function_call');
  assert.equal(converted.input[3].type, 'function_call_output');
  assert.equal(converted.tools[0].strict, false);
  assert.equal(converted.text.format.type, 'json_schema');
});

test('non-streaming responses preserve text and function calls', () => {
  const result = responseToChat({ id: 'resp_1', created_at: 7, status: 'completed', output: [
    { type: 'message', content: [{ type: 'output_text', text: 'hello' }] },
    { type: 'function_call', call_id: 'call1', name: 'lookup', arguments: '{}' },
  ], usage: { input_tokens: 3, output_tokens: 2, total_tokens: 5 } }, 'gpt-test');
  assert.equal(result.choices[0].message.content, 'hello');
  assert.equal(result.choices[0].message.tool_calls[0].id, 'call1');
  assert.equal(result.choices[0].finish_reason, 'tool_calls');
  assert.equal(result.usage.total_tokens, 5);
});

test('response events become incremental chat chunks', async () => {
  async function* events() {
    yield { type: 'response.output_text.delta', delta: 'hello' };
    yield { type: 'response.output_item.added', output_index: 1,
      item: { type: 'function_call', call_id: 'call1', name: 'lookup' } };
    yield { type: 'response.function_call_arguments.delta', output_index: 1, delta: '{}' };
    yield { type: 'response.completed', response: { output: [
      { type: 'function_call', call_id: 'call1', name: 'lookup', arguments: '{}' },
    ] } };
  }
  const chunks = [];
  for await (const chunk of responsesToChatStream(events(), 'gpt-test')) chunks.push(chunk);
  assert.equal(chunks[0].choices[0].delta.content, 'hello');
  assert.equal(chunks[1].choices[0].delta.tool_calls[0].id, 'call1');
  assert.equal(chunks[2].choices[0].delta.tool_calls[0].function.arguments, '{}');
  assert.equal(chunks.at(-2).choices[0].finish_reason, 'tool_calls');
  assert.equal(chunks.at(-1), '[DONE]');
});

test('invalid chat inputs return validation errors', () => {
  assert.throws(() => chatToResponses({ messages: [] }), /messages is required/);
  assert.throws(() => chatToResponses({ messages: [{ role: 'user', content: 'x' }], n: 2 }), /n must be 1/);
  assert.throws(() => chatToResponses({ messages: [{ role: 'user', content: 'x' }],
    response_format: { type: 'json_schema', json_schema: null } }), /must be an object/);
});


