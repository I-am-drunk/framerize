import { event } from './events.js';
import { replyBlocks, replyObject } from './completed-reply.js';

function optionalUsage(reply: Record<string, unknown>): Record<string, unknown> {
  return reply.usage === undefined || reply.usage === null
    ? { input_tokens: 0, output_tokens: 0 } : replyObject(reply.usage);
}

export function* messagesStream(reply: Record<string, unknown>): Generator<Buffer> {
  const usage = optionalUsage(reply);
  yield event('message_start', { message: { ...reply, content: [], stop_reason: null,
    stop_sequence: null, usage: { ...usage, output_tokens: 0 } } });
  for (const [index, block] of replyBlocks(reply.content).entries()) {
    const start = { ...block };
    const deltas: Record<string, unknown>[] = [];
    if (block.type === 'text') {
      start.text = '';
      deltas.push({ type: 'text_delta', text: block.text });
    } else if (block.type === 'tool_use' || block.type === 'server_tool_use') {
      start.input = {};
      deltas.push({ type: 'input_json_delta', partial_json: JSON.stringify(block.input) });
    } else if (block.type === 'thinking') {
      start.thinking = ''; start.signature = '';
      deltas.push({ type: 'thinking_delta', thinking: block.thinking },
        { type: 'signature_delta', signature: block.signature });
    }
    yield event('content_block_start', { index, content_block: start });
    for (const delta of deltas) yield event('content_block_delta', { index, delta });
    yield event('content_block_stop', { index });
  }
  yield event('message_delta', { delta: { stop_reason: reply.stop_reason,
    stop_sequence: reply.stop_sequence ?? null, ...(reply.stop_details === undefined ? {}
      : { stop_details: reply.stop_details }) }, usage });
  yield event('message_stop', {});
}
