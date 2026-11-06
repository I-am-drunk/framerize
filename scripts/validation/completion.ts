import { validateCompletedReply, type CompletedReplyProtocol }
  from '../../framerize-server/src/compat/completed-reply.js';

export function inspectJsonCompletion(protocol: CompletedReplyProtocol, bytes: Buffer) {
  validateCompletedReply(protocol, bytes);
  const reply = JSON.parse(bytes.toString('utf8')) as Record<string, unknown>;
  const blocks: Record<string, unknown>[] = [];
  if (protocol === 'responses') {
    if (reply.status !== 'completed') throw new Error('Incomplete text response');
    for (const item of reply.output as Record<string, unknown>[]) {
      if (item.status !== undefined && item.status !== 'completed') throw new Error('Incomplete item');
      if (item.type === 'reasoning') continue;
      if (item.type !== 'message') throw new Error('Not a text response');
      blocks.push(...item.content as Record<string, unknown>[]);
    }
  } else {
    if (reply.stop_reason !== 'end_turn') throw new Error('Incomplete text response');
    blocks.push(...reply.content as Record<string, unknown>[]);
  }
  const text: string[] = [];
  for (const block of blocks) {
    if (protocol === 'messages' && ['thinking', 'redacted_thinking'].includes(String(block.type))) continue;
    if (block.type !== (protocol === 'responses' ? 'output_text' : 'text')
      || typeof block.text !== 'string') throw new Error('Not a text response');
    text.push(block.text);
  }
  if (!text.some(value => value.trim())) throw new Error('Empty text response');
  return { contentBytes: Buffer.byteLength(text.join('')), textBlocks: text.length };
}

export function inspectCompletion(bytes: Buffer) {
  const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes).replace(/\r\n|\r/g, '\n');
  const frames = text.split('\n\n');
  const tail = frames.pop() ?? '';
  if (tail.split('\n').some(line => line.trim() && !line.startsWith(':'))) {
    throw new Error('Incomplete event');
  }
  let done = false;
  let finished = false;
  let events = 0;
  let contentBytes = 0;
  let textSeen = false;
  for (const frame of frames) {
    const lines = frame.split('\n');
    const dataLines = lines.filter(line => line === 'data' || line.startsWith('data:'));
    if (!dataLines.length) continue;
    const event = lines.filter(line => line === 'event' || line.startsWith('event:')).at(-1);
    if (event?.slice(6).replace(/^ /, '') === 'error') throw new Error('Error event');
    const data = dataLines
      .map(line => line.slice(5).replace(/^ /, '')).join('\n');
    if (done) throw new Error('Data after completion');
    if (data === '[DONE]') { done = true; continue; }
    const value = JSON.parse(data) as Record<string, unknown>;
    if (!value || typeof value !== 'object' || Object.hasOwn(value, 'error')
      || !Array.isArray(value.choices)) throw new Error('Invalid completion');
    events += 1;
    if (value.choices.length === 0) continue;
    if (value.choices.length !== 1 || finished) throw new Error('Unexpected choice');
    const choice = value.choices[0] as Record<string, unknown> | null;
    if (!choice || choice.index !== 0 || !choice.delta || typeof choice.delta !== 'object'
      || Array.isArray(choice.delta)) {
      throw new Error('Invalid choice');
    }
    const delta = choice.delta as Record<string, unknown>;
    if (delta.refusal || delta.tool_calls) throw new Error('Unexpected response type');
    if (delta.content !== undefined && delta.content !== null) {
      if (typeof delta.content !== 'string') throw new Error('Invalid content');
      contentBytes += Buffer.byteLength(delta.content);
      textSeen ||= delta.content.trim().length > 0;
    }
    if (choice.finish_reason !== undefined && choice.finish_reason !== null) {
      if (choice.finish_reason !== 'stop') throw new Error('Incomplete response');
      finished = true;
    }
  }
  if (!done || !finished || !textSeen) throw new Error('No completed text response');
  return { events, contentBytes };
}
