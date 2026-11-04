import assert from 'node:assert/strict';

function record(value: unknown): Record<string, unknown> {
  assert.ok(value && typeof value === 'object' && !Array.isArray(value));
  return value as Record<string, unknown>;
}

export function readCustomStream(protocol: 'responses' | 'messages', bytes: Buffer) {
  const events = bytes.toString().split('\n\n').flatMap(frame => {
    const data = frame.split('\n').find(line => line.startsWith('data: '));
    if (!data) return [];
    const value = record(JSON.parse(data.slice(6)));
    assert.ok(frame.includes(`event: ${value.type}\n`));
    assert.notEqual(value.type, 'error');
    return [value];
  });
  const terminal = protocol === 'responses' ? 'response.completed' : 'message_stop';
  assert.equal(events.at(-1)?.type, terminal);
  assert.equal(events.filter(event => event.type === terminal).length, 1);
  if (protocol === 'responses') {
    const reply = record(events.at(-1)?.response);
    const output = events.filter(event => event.type === 'response.output_item.done').map(event => event.item);
    assert.deepEqual(output, reply.output);
    for (const item of output.map(record).filter(item => item.type === 'function_call')) {
      const deltas = events.filter(event => event.type === 'response.function_call_arguments.delta'
        && event.item_id === item.id);
      assert.equal(deltas.map(event => event.delta).join(''), item.arguments);
    }
    return reply;
  }
  const reply = { ...record(events[0]?.message) };
  assert.equal(events[0]?.type, 'message_start');
  const content: Record<string, unknown>[] = [];
  for (const event of events) {
    if (event.type === 'content_block_start') {
      assert.equal(event.index, content.length);
      content.push({ ...record(event.content_block) });
    } else if (event.type === 'content_block_delta') {
      assert.equal(typeof event.index, 'number');
      const block = content[event.index as number];
      assert.ok(block);
      const delta = record(event.delta);
      if (delta.type === 'text_delta') block.text = String(block.text) + delta.text;
      else if (delta.type === 'input_json_delta') {
        block.partial = String(block.partial ?? '') + delta.partial_json;
      } else assert.fail('Unexpected delta in this text/tool fixture');
    } else if (event.type === 'message_delta') {
      Object.assign(reply, record(event.delta), { usage: event.usage });
    }
  }
  for (const block of content) {
    if (block.type !== 'tool_use') continue;
    assert.equal(typeof block.partial, 'string');
    block.input = JSON.parse(block.partial as string);
    delete block.partial;
  }
  assert.equal(events.filter(event => event.type === 'content_block_stop').length, content.length);
  return { ...reply, content };
}
