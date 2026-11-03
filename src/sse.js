export async function* parseSse(body, inspect, passThrough = false) {
  if (!body) throw new Error('upstream returned an empty body');
  const decoder = new TextDecoder();
  let buffer = '';
  let start = 0;
  for await (const chunk of body) {
    buffer += typeof chunk === 'string' ? chunk : decoder.decode(chunk, { stream: true });
    for (let index = start; index < buffer.length; index += 1) {
      if (!['\r', '\n'].includes(buffer[index])) continue;
      if (index === buffer.length - 1 && buffer[index] === '\r') break;
      const length = buffer[index] === '\r' && buffer[index + 1] === '\n' ? 2 : 1;
      if (!['\r', '\n'].includes(buffer[index + length])) {
        index += length - 1;
        continue;
      }
      const event = parseEvent(buffer.slice(0, index));
      buffer = buffer.slice(index + length + (buffer[index + length] === '\r'
        && buffer[index + length + 1] === '\n' ? 2 : 1));
      index = -1;
      start = 0;
      if (event) {
        inspect?.(event);
        if (!passThrough) yield event;
      }
    }
    start = Math.max(0, buffer.length - 1);
    if (passThrough) yield chunk;
  }
  buffer += decoder.decode();
}

function parseEvent(raw) {
  let event = 'message';
  const data = [];
  let id;
  for (const line of raw.split(/\r\n|\r|\n/)) {
    if (!line || line.startsWith(':')) continue;
    const colon = line.indexOf(':');
    const field = colon === -1 ? line : line.slice(0, colon);
    let value = colon === -1 ? '' : line.slice(colon + 1);
    if (value.startsWith(' ')) value = value.slice(1);
    if (field === 'event') event = value;
    if (field === 'data') data.push(value);
    if (field === 'id') id = value;
  }
  if (!data.length) return null;
  return { event, data: data.join('\n'), ...(id === undefined ? {} : { id }) };
}

export function encodeSse({ event = 'message', data = '', id }) {
  const lines = [];
  if (id !== undefined) lines.push(`id: ${String(id).replace(/[\r\n]/g, '')}`);
  if (event !== 'message') lines.push(`event: ${String(event).replace(/[\r\n]/g, '')}`);
  for (const line of String(data).split('\n')) lines.push(`data: ${line}`);
  return `${lines.join('\n')}\n\n`;
}

export function abortError(message = 'request aborted') {
  return new DOMException(message, 'AbortError');
}

export function createDeadline(signal, timeoutMs, onTimeout) {
  const controller = new AbortController();
  let timedOut = false;
  const timer = setTimeout(() => {
    timedOut = true;
    controller.abort(abortError('deadline exceeded'));
    onTimeout?.();
  }, timeoutMs);
  const abort = () => controller.abort(signal.reason);
  if (signal?.aborted) abort();
  else signal?.addEventListener('abort', abort, { once: true });
  return {
    signal: controller.signal,
    timedOut: () => timedOut,
    clear() {
      clearTimeout(timer);
      signal?.removeEventListener('abort', abort);
    },
  };
}

export async function* idleTimeout(iterable, timeoutMs, signal) {
  const iterator = iterable[Symbol.asyncIterator]();
  let completed = false;
  try {
    while (true) {
      let timer;
      let abort;
      const expired = new Promise((_, reject) => {
        timer = setTimeout(() => {
          const closing = iterator.return?.();
          if (closing?.catch) closing.catch(() => {});
          reject(new Error('upstream idle timeout'));
        }, timeoutMs);
      });
      const aborted = new Promise((_, reject) => {
        abort = () => reject(signal.reason);
        if (signal?.aborted) reject(signal.reason);
        else signal?.addEventListener('abort', abort, { once: true });
      });
      const pending = iterator.next();
      let result;
      try {
        result = await Promise.race([pending, expired, aborted]);
      } finally {
        clearTimeout(timer);
        signal?.removeEventListener('abort', abort);
      }
      if (result.done) {
        completed = true;
        return;
      }
      yield result.value;
    }
  } finally {
    if (!completed) {
      const closing = iterator.return?.();
      if (closing?.catch) closing.catch(() => {});
    }
  }
}
