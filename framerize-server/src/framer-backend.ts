import { ConfigurationError, ProviderError } from './errors.js';
import { createHttpProvider, type Provider } from './provider.js';
import { decodeFramerRequest, encodeFramerRequest, FRAMER_ROUTES } from './framer-request.js';
import type { FramerAuth } from './framer-auth.js';
import type { InferenceBackend, InferenceResponse, RequestContext } from '../dist/core.js';
import { CompletedReplyError, MAX_COMPLETED_REPLY_BYTES, validateCompletedReply,
  type CompletedReplyProtocol } from './compat/completed-reply.js';

export interface FramerMinter {
  mint(request: { readonly signal: AbortSignal; readonly cuid?: string }): Promise<{ readonly token: string }>;
  close(): Promise<void>;
}

export interface FramerBackendOptions {
  readonly projectId: string;
  readonly castleDeviceId?: string;
  readonly auth: FramerAuth;
  readonly castle: FramerMinter;
  readonly origin?: string;
  readonly maxBodyBytes?: number;
  readonly headersTimeoutMs?: number;
  readonly inferenceTimeoutMs?: number;
  readonly idleTimeoutMs?: number;
}

interface Operation {
  readonly controller: AbortController;
  readonly done: Promise<void>;
  transport?: Provider;
  preparing: boolean;
  deadline: number;
  finish(): Promise<void>;
  prepared(timeout: number): void;
  received(): void;
}

const failed = (code: string, status = 503) =>
  new ProviderError('Framer provider unavailable', { code, status });

function begin(active: Set<Operation>, signal: AbortSignal, timeout: number): Operation {
  const controller = new AbortController();
  let resolveDone!: () => void;
  let closing: Promise<void> | undefined;
  const done = new Promise<void>(resolve => { resolveDone = resolve; });
  const abort = () => {
    controller.abort(failed('request_cancelled', 499));
    if (!operation.preparing) void operation.finish();
  };
  const expire = () => controller.abort(failed('headers_timeout', 504));
  let timer = setTimeout(expire, timeout);
  const operation: Operation = { controller, done, preparing: true, deadline: performance.now() + timeout,
    prepared(timeout) {
      clearTimeout(timer); timer = setTimeout(expire, timeout);
      operation.deadline = performance.now() + timeout;
    },
    received() { clearTimeout(timer); operation.preparing = false; },
    finish() {
      closing ??= Promise.resolve().then(async () => {
        clearTimeout(timer);
        signal.removeEventListener('abort', abort);
        controller.abort(failed('request_cancelled', 499));
        try { await operation.transport?.close(); }
        finally { active.delete(operation); resolveDone(); }
      });
      return closing;
    },
  };
  active.add(operation);
  signal.addEventListener('abort', abort, { once: true });
  if (signal.aborted) abort();
  return operation;
}

function options(input: FramerBackendOptions) {
  try {
    const { projectId, castleDeviceId, auth, castle } = input;
    if (castleDeviceId !== undefined && (typeof castleDeviceId !== 'string'
      || !/^[A-Za-z0-9._~-]{1,256}$/.test(castleDeviceId))) throw new Error();
    const origin = new URL(input.origin ?? 'https://api.framer.com');
    const maxBodyBytes = input.maxBodyBytes ?? 16 * 1024 * 1024;
    const headersTimeoutMs = input.headersTimeoutMs ?? 15_000;
    const inferenceTimeoutMs = input.inferenceTimeoutMs ?? 300_000;
    const idleTimeoutMs = input.idleTimeoutMs ?? 60_000;
    if (typeof projectId !== 'string' || !/^[A-Za-z0-9_-]{1,128}$/.test(projectId) ||
        origin.username || origin.password || origin.search || origin.hash || origin.pathname !== '/' ||
        origin.protocol !== 'https:' && !(origin.protocol === 'http:' &&
          ['127.0.0.1', '[::1]'].includes(origin.hostname))) throw new Error();
    for (const value of [maxBodyBytes, headersTimeoutMs, inferenceTimeoutMs, idleTimeoutMs]) {
      if (!Number.isSafeInteger(value) || value < 1 || value > 2_147_483_647) throw new Error();
    }
    return { projectId, castleDeviceId, origin: origin.href,
      maxBodyBytes, headersTimeoutMs, inferenceTimeoutMs, idleTimeoutMs,
      accessToken: auth.accessToken.bind(auth), closeAuth: auth.close.bind(auth),
      mint: castle.mint.bind(castle), closeCastle: castle.close.bind(castle) };
  } catch { throw new ConfigurationError('Invalid Framer provider configuration'); }
}

async function collect(body: InferenceResponse['body'], signal: AbortSignal, limit: number,
  purpose: 'request' | 'reply' = 'request'): Promise<Buffer> {
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
      if (!Buffer.isBuffer(next.value)) throw purpose === 'reply'
        ? new CompletedReplyError() : failed('invalid_request_body', 400);
      size += next.value.length;
      if (size > limit) throw purpose === 'reply' ? new CompletedReplyError() : failed('body_too_large', 413);
      chunks.push(next.value);
    }
  } finally {
    signal.removeEventListener('abort', abort);
    if (!complete) void Promise.resolve().then(() => iterator.return?.()).catch(() => {});
  }
}

async function completedResponse(response: InferenceResponse, protocol: CompletedReplyProtocol,
  signal: AbortSignal): Promise<InferenceResponse> {
  const successful = response.status >= 200 && response.status < 300;
  if (successful) {
    const type = response.headers['content-type']?.split(';', 1)[0]?.trim().toLowerCase();
    const encoding = response.headers['content-encoding']?.trim().toLowerCase();
    if (!type || !/^application\/(?:[a-z0-9!#$&^_.+-]+\+)?json$/.test(type)
      || encoding !== undefined && encoding !== 'identity') throw new CompletedReplyError();
  }
  let bytes = await collect(response.body, signal, MAX_COMPLETED_REPLY_BYTES, 'reply');
  if (successful) validateCompletedReply(protocol, bytes);
  else {
    const types: Readonly<Record<number, string>> = {
      400: 'invalid_request_error', 401: 'authentication_error', 403: 'permission_error',
      404: 'not_found_error', 413: 'request_too_large', 429: 'rate_limit_error',
    };
    const error = { type: protocol === 'messages' ? types[response.status] ?? 'api_error' : 'upstream_error',
      code: 'upstream_http_error', message: 'Upstream request failed' };
    bytes = Buffer.from(JSON.stringify(protocol === 'messages' ? { type: 'error', error } : { error }));
  }
  signal.throwIfAborted();
  const headers: Record<string, string> = { ...response.headers, 'content-type': 'application/json' };
  if (!successful) headers['x-should-retry'] = 'false';
  delete headers['content-encoding'];
  delete headers['content-length'];
  return { status: response.status, headers, body: [bytes] };
}

function responseBody(body: InferenceResponse['body'], operation: Operation): AsyncIterableIterator<Buffer> {
  const iterator = (async function* () {
    try {
      for await (const chunk of body) {
        operation.controller.signal.throwIfAborted();
        yield chunk;
      }
      operation.controller.signal.throwIfAborted();
    } finally { await operation.finish(); }
  })();
  return {
    next: () => iterator.next(),
    async return() { await operation.finish(); return iterator.return(); },
    async throw() { await operation.finish(); return iterator.throw(failed('request_cancelled', 499)); },
    [Symbol.asyncIterator]() { return this; },
  };
}

async function accessToken(config: ReturnType<typeof options>, signal: AbortSignal): Promise<string> {
  try {
    const token = await config.accessToken({ signal });
    if (typeof token !== 'string' || token.length > 16_384 || !/^[A-Za-z0-9._~+/-]+=*$/.test(token)) {
      throw new Error();
    }
    return token;
  } catch { throw failed('credentials_unavailable'); }
}

function mintFailure(error: unknown): ProviderError {
  try {
    if ((error as { code?: unknown } | null)?.code === 'busy') return failed('provider_busy');
  } catch {}
  return failed('credentials_unavailable');
}

export function createFramerBackend(input: FramerBackendOptions): InferenceBackend {
  const config = options(input);
  const active = new Set<Operation>();
  let closing: Promise<void> | undefined;
  function start(context: RequestContext) {
    if (closing) throw failed('provider_closed');
    let signal: AbortSignal;
    try {
      signal = context.signal;
      if (!(signal instanceof AbortSignal)) throw new Error();
      AbortSignal.prototype.throwIfAborted.call(signal);
    } catch { throw failed('request_cancelled', 499); }
    return begin(active, signal, config.headersTimeoutMs);
  }
  return {
    async ready(context) {
      const operation = start(context);
      const { signal } = operation.controller;
      try {
        await accessToken(config, signal);
        signal.throwIfAborted();
        return true;
      } catch {
        signal.throwIfAborted();
        return { ready: false, reason: 'credentials_unavailable' };
      } finally { await operation.finish(); }
    },
    async forward(request) {
      const operation = start(request);
      const { signal } = operation.controller;
      let completed = false;
      let submitted = false;
      try {
        const bytes = await collect(request.body, signal, config.maxBodyBytes);
        const decoded = decodeFramerRequest(request.path, bytes);
        completed = decoded.completedProtocol !== undefined;
        const authorization = await accessToken(config, signal);
        signal.throwIfAborted();
        const castleToken = await Promise.resolve().then(async () => (await config.mint({ signal,
          ...(config.castleDeviceId === undefined ? {} : { cuid: config.castleDeviceId }),
        })).token)
          .catch((error: unknown) => { throw mintFailure(error); });
        signal.throwIfAborted();
        const encoded = encodeFramerRequest(decoded, config.projectId, castleToken);
        operation.prepared(config.inferenceTimeoutMs);
        operation.transport = createHttpProvider({ upstream: config.origin, authorization,
          headersTimeoutMs: config.headersTimeoutMs, inferenceTimeoutMs: config.inferenceTimeoutMs,
          idleTimeoutMs: config.idleTimeoutMs }, FRAMER_ROUTES);
        submitted = true;
        const response = await operation.transport.forward({ path: request.path, signal,
          headers: { ...(completed ? request.headers : {}),
            accept: completed ? 'application/json' : 'text/event-stream, application/json',
            ...(completed ? { 'accept-encoding': 'identity' } : {}), 'content-type': 'application/json' },
          body: (async function* () { yield encoded; })(),
        });
        signal.throwIfAborted();
        if (decoded.completedProtocol) {
          const result = await completedResponse(response, decoded.completedProtocol, signal);
          if (performance.now() >= operation.deadline) operation.controller.abort(failed('headers_timeout', 504));
          signal.throwIfAborted();
          await operation.finish();
          return result;
        }
        operation.received();
        return { ...response, body: responseBody(response.body, operation) };
      } catch (error) {
        const reason: unknown = signal.aborted ? signal.reason : error;
        await operation.finish();
        if (completed && submitted) throw new ProviderError('Completed request failed', {
          code: reason instanceof ProviderError ? reason.code : 'upstream_error',
          status: reason instanceof ProviderError ? reason.status : 502, retryable: false,
        });
        throw reason instanceof ProviderError ? reason : failed('upstream_error', 502);
      }
    },
    close() {
      closing ??= Promise.resolve().then(async () => {
        const operations = [...active];
        for (const operation of operations) {
          operation.controller.abort(failed('provider_closed'));
          if (!operation.preparing) void operation.finish();
        }
        const results = await Promise.allSettled([
          Promise.resolve().then(config.closeAuth), Promise.resolve().then(config.closeCastle),
          ...operations.map(operation => operation.done),
        ]);
        if (results.some(result => result.status === 'rejected')) throw failed('provider_close_failed');
      });
      return closing;
    },
  };
}
