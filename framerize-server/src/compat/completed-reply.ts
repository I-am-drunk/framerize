import { ProviderError } from '../errors.js';

export type CompletedReplyProtocol = 'responses' | 'messages';
export const MAX_COMPLETED_REPLY_BYTES = 16 * 1024 * 1024;

export class CompletedReplyError extends ProviderError {
  readonly retryable = false;

  constructor() {
    super('Upstream returned an invalid completed reply', {
      code: 'upstream_response_invalid', status: 502,
    });
    this.name = 'CompletedReplyError';
  }
}

function invalid(): never { throw new CompletedReplyError(); }

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) invalid();
  return value as Record<string, unknown>;
}

function nonempty(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0;
}

function blocks(value: unknown): Record<string, unknown>[] {
  if (!Array.isArray(value)) invalid();
  return value.map(item => {
    const block = object(item);
    if (!nonempty(block.type)) invalid();
    return block;
  });
}

function usage(value: unknown): void {
  if (value === undefined || value === null) return;
  const details = object(value);
  for (const field of ['input_tokens', 'output_tokens', 'total_tokens']) {
    if (field === 'total_tokens' && details[field] === undefined) continue;
    const count = details[field];
    if (typeof count !== 'number' || !Number.isSafeInteger(count) || count < 0) invalid();
  }
}

function content(block: Record<string, unknown>): void {
  if (block.type === 'text' || block.type === 'output_text') {
    if (typeof block.text !== 'string') invalid();
  } else if (block.type === 'refusal') {
    if (typeof block.refusal !== 'string') invalid();
  } else if (block.type === 'thinking') {
    if (typeof block.thinking !== 'string' || !nonempty(block.signature)) invalid();
  } else if (block.type === 'redacted_thinking') {
    if (!nonempty(block.data)) invalid();
  } else if (block.type === 'tool_use' || block.type === 'server_tool_use') {
    if (!nonempty(block.id) || !nonempty(block.name)) invalid();
    object(block.input);
  }
}

function responsesReply(reply: Record<string, unknown>): void {
  if (reply.object !== 'response' || (reply.status !== 'completed' && reply.status !== 'incomplete')
    || typeof reply.created_at !== 'number' || !Number.isFinite(reply.created_at)
    || reply.created_at < 0) invalid();
  if (reply.status === 'incomplete') {
    if (!nonempty(object(reply.incomplete_details).reason)) invalid();
  } else if (reply.incomplete_details !== undefined && reply.incomplete_details !== null) invalid();
  for (const item of blocks(reply.output)) {
    if (item.status !== undefined && item.status !== 'completed' && item.status !== 'failed'
      && item.status !== 'cancelled' && !(reply.status === 'incomplete' && item.status === 'incomplete')) invalid();
    if (item.type === 'message') {
      if (item.role !== 'assistant' || !nonempty(item.id)) invalid();
      for (const block of blocks(item.content)) content(block);
    } else if (item.type === 'function_call' || item.type === 'custom_tool_call') {
      if (!nonempty(item.call_id) || !nonempty(item.name)) invalid();
      if (item.status !== undefined && item.status !== 'completed'
        && !(reply.status === 'incomplete' && item.status === 'incomplete')) invalid();
      if (item.id !== undefined && !nonempty(item.id)) invalid();
      if (item.namespace !== undefined && !nonempty(item.namespace)) invalid();
      if (item.type === 'function_call') {
        if (typeof item.arguments !== 'string') invalid();
        if (item.status !== 'incomplete') JSON.parse(item.arguments);
      } else if (typeof item.input !== 'string') invalid();
    }
  }
}

const MESSAGE_STOPS = new Set(['end_turn', 'max_tokens', 'stop_sequence', 'tool_use',
  'pause_turn', 'refusal', 'model_context_window_exceeded']);

function messagesReply(reply: Record<string, unknown>): void {
  if (reply.type !== 'message' || reply.role !== 'assistant'
    || typeof reply.stop_reason !== 'string' || !MESSAGE_STOPS.has(reply.stop_reason)) invalid();
  if (reply.stop_sequence !== undefined && reply.stop_sequence !== null
    && typeof reply.stop_sequence !== 'string') invalid();
  if (reply.stop_reason === 'stop_sequence' && typeof reply.stop_sequence !== 'string') invalid();
  const items = blocks(reply.content);
  if (reply.stop_reason === 'tool_use'
    && !items.some(item => item.type === 'tool_use' || item.type === 'server_tool_use')) invalid();
  for (const block of items) content(block);
}

export function validateCompletedReply(protocol: CompletedReplyProtocol, bytes: Uint8Array): void {
  try {
    if (!(bytes instanceof Uint8Array) || bytes.byteLength === 0
      || bytes.byteLength > MAX_COMPLETED_REPLY_BYTES) invalid();
    const reply = object(JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)));
    if (!nonempty(reply.id) || !nonempty(reply.model)
      || (reply.error !== undefined && reply.error !== null)) invalid();
    if (protocol === 'responses') responsesReply(reply);
    else if (protocol === 'messages') messagesReply(reply);
    else invalid();
    usage(reply.usage);
  } catch { throw new CompletedReplyError(); }
}
