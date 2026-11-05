import type { InferenceResponse } from '../../dist/core.js';
import { gunzipSync } from 'node:zlib';
import { ProviderError } from '../errors.js';
import { event } from './events.js';
import { messagesStream } from './messages-stream.js';
import { responsesStream } from './responses-stream.js';
import { CompletedReplyError, type CompletedReplyProtocol } from './completed-reply.js';

export interface CompletedResponse extends InferenceResponse {
  readonly reply?: Record<string, unknown>;
}

type Outcome = { response: CompletedResponse } | { error: unknown };

function streamError(protocol: CompletedReplyProtocol, error: unknown): Buffer {
  const known = error instanceof ProviderError ? error : undefined;
  const code = known && ['upstream_http_error', 'upstream_response_invalid', 'headers_timeout',
    'idle_timeout'].includes(known.code) ? known.code : 'upstream_error';
  const message = 'Upstream request failed';
  return protocol === 'responses'
    ? event('error', { code, message, param: null, sequence_number: 0 })
    : event('error', { error: { type: errorTypes[known?.status ?? 502] ?? 'api_error', code, message } });
}

const errorTypes: Readonly<Record<number, string>> = {
  400: 'invalid_request_error', 401: 'authentication_error', 403: 'permission_error',
  404: 'not_found_error', 413: 'request_too_large', 429: 'rate_limit_error', 529: 'overloaded_error',
};

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown> : undefined;
}

function errorValue(bytes: Buffer, encoding: string): unknown {
  try {
    if (encoding !== 'identity' && encoding !== 'gzip') throw new Error();
    const decoded = encoding === 'gzip' ? gunzipSync(bytes, { maxOutputLength: 65_536 }) : bytes;
    const text = new TextDecoder('utf-8', { fatal: true }).decode(decoded);
    try { return JSON.parse(text) as unknown; } catch { return text; }
  } catch {
    return { encoding: 'base64', content_encoding: encoding, data: bytes.toString('base64') };
  }
}

async function errorBytes(body: InferenceResponse['body'], signal: AbortSignal): Promise<Buffer> {
  signal.throwIfAborted();
  const asynchronous = body as AsyncIterable<Buffer>;
  const iterator = typeof asynchronous[Symbol.asyncIterator] === 'function'
    ? asynchronous[Symbol.asyncIterator]() : (body as Iterable<Buffer>)[Symbol.iterator]();
  const chunks: Buffer[] = [];
  let size = 0;
  let complete = false;
  let abort!: () => void;
  const stopped = new Promise<never>((resolve, reject) => {
    abort = () => reject(signal.reason);
    signal.addEventListener('abort', abort, { once: true });
    if (signal.aborted) abort();
  });
  try {
    while (true) {
      const next = await Promise.race([Promise.resolve().then(() => iterator.next()), stopped]);
      signal.throwIfAborted();
      if (next.done) { complete = true; return Buffer.concat(chunks, size); }
      if (!Buffer.isBuffer(next.value) || next.value.length > 65_536 - size) throw new CompletedReplyError();
      size += next.value.length;
      chunks.push(next.value);
    }
  } finally {
    signal.removeEventListener('abort', abort);
    if (!complete) void Promise.resolve().then(() => iterator.return?.()).catch(() => {});
  }
}

async function httpError(response: CompletedResponse, protocol: CompletedReplyProtocol,
  signal: AbortSignal): Promise<Buffer> {
  const value = errorValue(await errorBytes(response.body, signal),
    response.headers['content-encoding']?.trim().toLowerCase() ?? 'identity');
  signal.throwIfAborted();
  const envelope = record(value);
  const details = record(envelope?.error) ?? envelope;
  const message = typeof envelope?.error === 'string' ? envelope.error
    : typeof value === 'string' && value ? value : 'Upstream request failed';
  const error = { type: errorTypes[response.status] ?? 'api_error', code: 'upstream_http_error',
    message, ...details };
  return protocol === 'responses'
    ? event('error', { param: null, ...error, upstream: value, sequence_number: 0 })
    : event('error', { error, upstream: value });
}

export function streamCompletedReply(pending: Promise<CompletedResponse>, protocol: CompletedReplyProtocol,
  signal: AbortSignal, heartbeatMs: number): AsyncGenerator<Buffer> {
  let outcome: Outcome | undefined;
  let wake = () => {};
  void pending.then(response => { outcome = { response }; wake(); }, error => { outcome = { error }; wake(); });
  return (async function* () {
    try {
      while (!outcome) {
        signal.throwIfAborted();
        yield Buffer.from(': keepalive\n\n');
        await new Promise<void>(resolve => {
          const finish = () => {
            clearTimeout(timer); signal.removeEventListener('abort', finish);
            wake = () => {}; resolve();
          };
          const timer = setTimeout(finish, heartbeatMs);
          wake = finish;
          signal.addEventListener('abort', finish, { once: true });
          if (outcome || signal.aborted) finish();
        });
      }
      signal.throwIfAborted();
      if ('error' in outcome) throw outcome.error;
      const { response } = outcome;
      if (!response.reply) {
        yield await httpError(response, protocol, signal);
        return;
      }
      yield* protocol === 'responses' ? responsesStream(response.reply) : messagesStream(response.reply);
    } catch (error) {
      signal.throwIfAborted();
      yield streamError(protocol, error);
    }
  })();
}
