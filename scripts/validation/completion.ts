export function inspectCompletion(bytes: Buffer) {
  const text = new TextDecoder('utf-8', { fatal: true }).decode(bytes).replace(/\r\n|\r/g, '\n');
  const frames = text.split('\n\n');
  if (frames.pop()?.trim()) throw new Error('Incomplete event');
  let done = false;
  let finished = false;
  let events = 0;
  let contentBytes = 0;
  let textSeen = false;
  for (const frame of frames) {
    const lines = frame.split('\n');
    if (lines.some(line => /^event: ?error$/.test(line))) throw new Error('Error event');
    const data = lines.filter(line => line.startsWith('data:'))
      .map(line => line.slice(5).replace(/^ /, '')).join('\n');
    if (!data) continue;
    if (done) throw new Error('Data after completion');
    if (data === '[DONE]') { done = true; continue; }
    const value = JSON.parse(data) as Record<string, unknown>;
    if (!value || typeof value !== 'object' || Object.hasOwn(value, 'error')
      || !Array.isArray(value.choices)) throw new Error('Invalid completion');
    events += 1;
    if (value.choices.length === 0) continue;
    if (value.choices.length !== 1 || finished) throw new Error('Unexpected choice');
    const choice = value.choices[0] as Record<string, unknown> | null;
    if (!choice || choice.index !== 0 || !choice.delta || typeof choice.delta !== 'object') {
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
