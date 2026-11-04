import { randomUUID } from 'node:crypto';
import { event } from './events.js';
import { CompletedReplyError, replyBlocks, replyObject } from './completed-reply.js';

type Emit = (type: string, fields: Record<string, unknown>) => Buffer;

function* reasoningEvents(item: Record<string, unknown>, fields: Record<string, unknown>, emit: Emit): Generator<Buffer> {
  for (const [summary_index, part] of replyBlocks(item.summary).entries()) {
    const location = { ...fields, summary_index };
    yield emit('response.reasoning_summary_part.added', { ...location, part: { ...part, text: '' } });
    yield emit('response.reasoning_summary_text.delta', { ...location, delta: part.text });
    yield emit('response.reasoning_summary_text.done', { ...location, text: part.text });
    yield emit('response.reasoning_summary_part.done', { ...location, part });
  }
}

function* itemEvents(item: Record<string, unknown>, fields: Record<string, unknown>, emit: Emit): Generator<Buffer> {
  if (item.type === 'message') {
    for (const [content_index, part] of replyBlocks(item.content).entries()) {
      const location = { ...fields, content_index };
      const field = part.type === 'output_text' ? 'text' : part.type === 'refusal' ? 'refusal' : undefined;
      yield emit('response.content_part.added', { ...location,
        part: field ? { ...part, [field]: '', ...(field === 'text' ? { annotations: [] } : {}) } : part });
      if (field) {
        const name = `response.${part.type}`;
        yield emit(`${name}.delta`, { ...location, delta: part[field], ...(field === 'text' ? { logprobs: [] } : {}) });
        if (field === 'text' && Array.isArray(part.annotations)) {
          for (const [annotation_index, annotation] of part.annotations.entries()) {
            yield emit('response.output_text.annotation.added', { ...location, annotation_index, annotation });
          }
        }
        yield emit(`${name}.done`, { ...location, [field]: part[field], ...(field === 'text' ? { logprobs: [] } : {}) });
      }
      yield emit('response.content_part.done', { ...location, part });
    }
  } else if (item.type === 'function_call' || item.type === 'custom_tool_call') {
    const field = item.type === 'function_call' ? 'arguments' : 'input';
    const name = item.type === 'function_call' ? 'response.function_call_arguments' : 'response.custom_tool_call_input';
    yield emit(`${name}.delta`, { ...fields, delta: item[field] });
    yield emit(`${name}.done`, { ...fields, [field]: item[field] });
  } else if (item.type === 'reasoning') {
    yield* reasoningEvents(item, fields, emit);
  }
}

export function* responsesStream(reply: Record<string, unknown>): Generator<Buffer> {
  let sequence = 0;
  const emit: Emit = (type, fields) => event(type, { ...fields, sequence_number: sequence++ });
  const output: Record<string, unknown>[] = replyBlocks(reply.output).map(item => ({ ...item,
    id: typeof item.id === 'string' && item.id ? item.id : `item_${randomUUID()}` }));
  const result: Record<string, unknown> = { ...reply, output };
  if (reply.usage !== undefined && reply.usage !== null) {
    const usage = replyObject(reply.usage);
    if (usage.total_tokens === undefined) {
      const total = Number(usage.input_tokens) + Number(usage.output_tokens);
      if (!Number.isSafeInteger(total)) throw new CompletedReplyError();
      result.usage = { ...usage, total_tokens: total };
    }
  }
  for (const item of output) {
    if (item.type !== 'reasoning') continue;
    for (const part of replyBlocks(item.summary)) {
      if (part.type !== 'summary_text' || typeof part.text !== 'string') throw new CompletedReplyError();
    }
  }
  const progress = { ...reply, output: [], status: 'in_progress', completed_at: null, usage: null,
    error: null, incomplete_details: null };
  yield emit('response.created', { response: progress });
  yield emit('response.in_progress', { response: progress });
  for (const [output_index, item] of output.entries()) {
    const fields = { output_index, item_id: item.id };
    const start: Record<string, unknown> = { ...item, status: 'in_progress' };
    if (item.type === 'message') start.content = [];
    else if (item.type === 'function_call') start.arguments = '';
    else if (item.type === 'custom_tool_call') start.input = '';
    else if (item.type === 'reasoning') start.summary = [];
    yield emit('response.output_item.added', { output_index, item: start });
    yield* itemEvents(item, fields, emit);
    yield emit('response.output_item.done', { output_index, item });
  }
  yield emit(reply.status === 'incomplete' ? 'response.incomplete' : 'response.completed',
    { response: result });
}
