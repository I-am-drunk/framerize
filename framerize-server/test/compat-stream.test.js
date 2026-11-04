import test from 'node:test';
import assert from 'node:assert/strict';
import { parseCompletedReply } from '../dist/compat/completed-reply.js';
import { responsesStream } from '../dist/compat/responses-stream.js';
import { messagesStream } from '../dist/compat/messages-stream.js';

const common = { id: 'reply', model: 'model', usage: { input_tokens: 17, output_tokens: 29 } };
const responses = output => ({ ...common, usage: { ...common.usage, total_tokens: 46 },
  object: 'response', status: 'completed', created_at: 1, output });
const messages = content => ({ ...common, type: 'message', role: 'assistant',
  stop_reason: 'end_turn', stop_sequence: null, content });

test('Responses derives a missing total from supplied usage without inventing token counts', () => {
  const value = { ...responses([]), usage: { input_tokens: 17, output_tokens: 29 } };
  const result = render('responses', value);
  assert.deepEqual(result.at(-1).response.usage, { ...value.usage, total_tokens: 46 });
  assert.equal(Object.hasOwn(value.usage, 'total_tokens'), false);
  const supplied = { ...value, usage: { ...value.usage, total_tokens: 50 } };
  assert.equal(render('responses', supplied).at(-1).response.usage.total_tokens, 50);
  assert.throws(() => render('responses', { ...value,
    usage: { input_tokens: Number.MAX_SAFE_INTEGER, output_tokens: 1 } }),
  { code: 'upstream_response_invalid' });
});

for (const type of ['function_call', 'custom_tool_call']) for (const incomplete of [false, true]) {
  test(`Responses ${type} preserves ${incomplete ? 'partial' : 'complete'} arguments and identity`, () => {
    const field = type === 'function_call' ? 'arguments' : 'input';
    const value = responses([{ type, call_id: 'call', name: 'tool', namespace: 'tools',
      status: incomplete ? 'incomplete' : 'completed', [field]: incomplete ? '{"query":' : '{"query":"snow"}' }]);
    if (incomplete) Object.assign(value, { status: 'incomplete', incomplete_details: { reason: 'max_output_tokens' } });
    const result = render('responses', value);
    const added = result.find(item => item.type === 'response.output_item.added');
    const done = result.find(item => item.type === 'response.output_item.done');
    const delta = result.find(item => item.type.endsWith('.delta'));
    assert.equal(added.item[field], '');
    assert.equal(delta.delta, value.output[0][field]);
    assert.equal(added.item.id, done.item.id);
    assert.equal(delta.item_id, done.item.id);
    assert.deepEqual(done.item, { ...value.output[0], id: added.item.id });
    assert.equal(result.at(-1).type, incomplete ? 'response.incomplete' : 'response.completed');
    assert.deepEqual(result.at(-1).response.incomplete_details, value.incomplete_details);
  });
}

test('Responses preserves reasoning summaries and encrypted state', () => {
  const value = responses([{ type: 'reasoning', id: 'reasoning', encrypted_content: 'opaque-state',
    summary: [{ type: 'summary_text', text: 'Plan.' }] }]);
  const result = render('responses', value);
  assert.equal(result.find(item => item.type === 'response.reasoning_summary_text.delta').delta, 'Plan.');
  assert.deepEqual(result.at(-1).response, value);
});
function render(protocol, value) {
  const reply = parseCompletedReply(protocol, Buffer.from(JSON.stringify(value)));
  const result = [...(protocol === 'responses' ? responsesStream(reply) : messagesStream(reply))]
    .map(bytes => JSON.parse(bytes.toString().split('\ndata: ')[1]));
  if (protocol === 'responses') assert.deepEqual(result.map(item => item.sequence_number), result.map((entry, index) => index));
  return result;
}

for (const reason of ['end_turn', 'max_tokens', 'tool_use', 'refusal', 'pause_turn', 'model_context_window_exceeded']) {
  test(`Messages preserves rich blocks, usage and ${reason}`, () => {
    const value = { ...messages([
      { type: 'thinking', thinking: 'Reasoning.', signature: 'signed' },
      { type: 'text', text: '雪\n🦊', citations: [{ type: 'page_location', cited_text: 'snow' }] },
      { type: 'tool_use', id: 'call-1', name: 'lookup', input: { query: 'snow' } },
      { type: 'tool_use', id: 'call-2', name: 'lookup', input: { query: 'rain' } },
      { type: 'redacted_thinking', data: 'opaque' },
    ]), stop_reason: reason, usage: { ...common.usage, cache_read_input_tokens: 7 } };
    const result = render('messages', value);
    assert.equal(result[0].type, 'message_start');
    assert.deepEqual(result[0].message.content, []);
    assert.equal(result[0].message.usage.input_tokens, 17);
    assert.equal(result.at(-1).type, 'message_stop');
    assert.equal(result.at(-2).delta.stop_reason, reason);
    assert.deepEqual(result.at(-2).usage, value.usage);
    assert.deepEqual(result.filter(item => item.type === 'content_block_start').map(item => item.index), [0, 1, 2, 3, 4]);
    assert.equal(result.filter(item => item.type === 'content_block_stop').length, 5);
    assert.deepEqual(result.filter(item => item.delta?.type === 'input_json_delta')
      .map(item => JSON.parse(item.delta.partial_json)), [{ query: 'snow' }, { query: 'rain' }]);
    assert.equal(result.find(item => item.delta?.type === 'signature_delta').delta.signature, 'signed');
  });
}

test('Responses preserves text, annotations, refusals, usage and response metadata', () => {
  const value = responses([{ type: 'message', role: 'assistant', id: 'message', status: 'completed',
    content: [{ type: 'output_text', text: '雪\n🦊', annotations: [{ type: 'url_citation', url: 'https://example.com' }] },
      { type: 'refusal', refusal: 'No.' }] }]);
  const result = render('responses', value);
  assert.equal(result[0].type, 'response.created');
  assert.deepEqual(result[0].response.output, []);
  assert.equal(result.find(item => item.type === 'response.output_text.delta').delta, '雪\n🦊');
  assert.equal(result.find(item => item.type === 'response.refusal.delta').delta, 'No.');
  assert.equal(result.filter(item => item.type === 'response.output_text.annotation.added').length, 1);
  assert.deepEqual(result.at(-1).response, value);
});
