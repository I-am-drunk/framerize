import { ConfigurationError, ProviderError } from './errors.js';
import { createHttpProvider, type Provider } from './provider.js';
import { decodeFramerRequest, encodeFramerRequest, FRAMER_ROUTES } from './framer-request.js';
import type { FramerAuth } from './framer-auth.js';
import type { InferenceBackend, InferenceResponse, RequestContext } from '../dist/core.js';

export interface FramerMinter {
  mint(request: { readonly signal: AbortSignal }): Promise<{ readonly token: string }>;
  close(): Promise<void>;
}

export interface FramerBackendOptions {
  readonly projectId: string;
  readonly auth: FramerAuth;
  readonly castle: FramerMinter;
  readonly origin?: string;
  readonly maxBodyBytes?: number;
  readonly headersTimeoutMs?: number;
  readonly idleTimeoutMs?: number;
}

interface Operation {
  readonly controller: AbortController;
  readonly done: Promise<void>;
  transport?: Provider;
  preparing: boolean;
  finish(): Promise<void>;
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
  const timer = setTimeout(() => controller.abort(failed('headers_timeout', 504)), timeout);
  const operation: Operation = { controller, done, preparing: true,
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
    const { projectId, auth, castle } = input;
    const origin = new URL(input.origin ?? 'https://api.framer.com');
    const maxBodyBytes = input.maxBodyBytes ?? 16 * 1024 * 1024;
    const headersTimeoutMs = input.headersTimeoutMs ?? 15_000;
    const idleTimeoutMs = input.idleTimeoutMs ?? 60_000;
    if (typeof projectId !== 'string' || !/^[A-Za-z0-9_-]{1,128}$/.test(projectId) ||
        origin.username || origin.password || origin.search || origin.hash || origin.pathname !== '/' ||
        origin.protocol !== 'https:' && !(origin.protocol === 'http:' &&
          ['127.0.0.1', '[::1]'].includes(origin.hostname))) throw new Error();
    for (const value of [maxBodyBytes, headersTimeoutMs, idleTimeoutMs]) {
      if (!Number.isSafeInteger(value) || value < 1 || value > 2_147_483_647) throw new Error();
    }
    return { projectId, origin: origin.href, maxBodyBytes, headersTimeoutMs, idleTimeoutMs,
      accessToken: auth.accessToken.bind(auth), closeAuth: auth.close.bind(auth),
      mint: castle.mint.bind(castle), closeCastle: castle.close.bind(castle) };
  } catch { throw new ConfigurationError('Invalid Framer provider configuration'); }
}

async function collect(body: AsyncIterable<Buffer>, signal: AbortSignal, limit: number): Promise<Buffer> {
  signal.throwIfAborted();
  const iterator = body[Symbol.asyncIterator]();
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
      if (!Buffer.isBuffer(next.value)) throw failed('invalid_request_body', 400);
      size += next.value.length;
      if (size > limit) throw failed('body_too_large', 413);
      chunks.push(next.value);
    }
  } finally {
    signal.removeEventListener('abort', abort);
    if (!complete) void Promise.resolve().then(() => iterator.return?.()).catch(() => {});
  }
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
      try {
        const bytes = await collect(request.body, signal, config.maxBodyBytes);
        const decoded = decodeFramerRequest(request.path, bytes);
        const authorization = await accessToken(config, signal);
        signal.throwIfAborted();
        const castleToken = await Promise.resolve().then(async () => (await config.mint({ signal })).token)
          .catch(() => { throw failed('credentials_unavailable'); });
        signal.throwIfAborted();
        const encoded = encodeFramerRequest(decoded, config.projectId, castleToken);
        operation.transport = createHttpProvider({ upstream: config.origin, authorization,
          headersTimeoutMs: config.headersTimeoutMs, idleTimeoutMs: config.idleTimeoutMs }, FRAMER_ROUTES);
        const response = await operation.transport.forward({ path: request.path, signal,
          headers: { accept: 'text/event-stream, application/json', 'content-type': 'application/json' },
          body: (async function* () { yield encoded; })(),
        });
        signal.throwIfAborted();
        operation.received();
        return { ...response, body: responseBody(response.body, operation) };
      } catch (error) {
        const aborted = signal.aborted;
        await operation.finish();
        if (aborted) throw signal.reason;
        throw error instanceof ProviderError ? error : failed('upstream_error', 502);
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
