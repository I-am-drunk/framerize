import http from 'node:http';
import https from 'node:https';
import { once } from 'node:events';
import type { IncomingMessage } from 'node:http';
import { RemoteDiagnosticError, networkDiagnostic, type DiagnosticCode } from './diagnostics.js';

export interface RawRequest {
  readonly origin: string;
  readonly path: string;
  readonly method: 'GET' | 'POST';
  readonly token: string;
  readonly headers: Readonly<Record<string, string>>;
  readonly body?: AsyncIterable<Buffer> | undefined;
  readonly signal: AbortSignal;
  readonly headersTimeoutMs: number;
  readonly connectTimeoutMs?: number;
  readonly idleTimeoutMs: number;
}

export interface RawResponse { readonly message: IncomingMessage; dispose(): void; }

export function sendRequest(request: RawRequest): Promise<RawResponse> {
  return new Promise((resolve, reject) => {
    if (request.signal.aborted) { reject(new RemoteDiagnosticError('aborted')); return; }
    const url = new URL(request.origin + request.path);
    const transport = url.protocol === 'https:' ? https : http;
    const agentOptions = { keepAlive: false, proxyEnv: {} };
    const agent = new transport.Agent(agentOptions);
    const controller = new AbortController();
    let outgoing: http.ClientRequest;
    let message: IncomingMessage | undefined;
    let timer: NodeJS.Timeout;
    let connectTimer: NodeJS.Timeout;
    let disposed = false;
    const dispose = (): void => {
      if (disposed) return;
      disposed = true;
      clearTimeout(timer);
      clearTimeout(connectTimer);
      request.signal.removeEventListener('abort', onAbort);
      controller.abort();
      message?.destroy();
      outgoing?.destroy();
      agent.destroy();
    };
    const fail = (code: DiagnosticCode): void => {
      if (disposed) return;
      const error = new RemoteDiagnosticError(code);
      message?.destroy(error);
      dispose(); reject(error);
    };
    const onError = (error: unknown): void => fail(networkDiagnostic(error));
    const onAbort = (): void => fail('aborted');
    try {
      outgoing = transport.request(url, {
        agent, method: request.method, maxHeaderSize: 16384,
        ...(url.protocol === 'https:' ? { rejectUnauthorized: true } : {}),
        headers: { ...request.headers, authorization: `Bearer ${request.token}` },
      });
      outgoing.maxHeadersCount = 0;
      outgoing.on('error', onError);
      outgoing.setTimeout(request.idleTimeoutMs, () => fail('idle_timeout'));
      connectTimer = setTimeout(() => fail('headers_timeout'), request.connectTimeoutMs ?? request.headersTimeoutMs);
      outgoing.once('socket', (socket) => {
        const connected = (): void => { clearTimeout(connectTimer); };
        if (socket.connecting) socket.once(url.protocol === 'https:' ? 'secureConnect' : 'connect', connected);
        else connected();
      });
      outgoing.once('finish', () => {
        if (disposed || message) return;
        outgoing.setTimeout(0);
        timer = setTimeout(() => fail('headers_timeout'), request.headersTimeoutMs);
      });
      request.signal.addEventListener('abort', onAbort, { once: true });
      outgoing.once('response', (response) => {
        if (disposed) { response.destroy(); return; }
        message = response;
        clearTimeout(timer);
        clearTimeout(connectTimer);
        response.setTimeout(request.idleTimeoutMs, () => fail('idle_timeout'));
        response.once('error', onError);
        resolve({ message: response, dispose });
      });
      void pump(outgoing, request.body, controller.signal).catch(onError);
    } catch (error) { onError(error); }
  });
}

async function pump(
  outgoing: http.ClientRequest, body: AsyncIterable<Buffer> | undefined, signal: AbortSignal,
): Promise<void> {
  if (body) {
    const iterator = body[Symbol.asyncIterator]();
    let complete = false;
    try {
      while (!signal.aborted) {
        const item = await nextItem(iterator, signal);
        if (item.done) { complete = true; break; }
        if (!outgoing.write(item.value)) await once(outgoing, 'drain', { signal });
      }
    } finally {
      if (!complete) {
        try { void Promise.resolve(iterator.return?.()).catch(() => {}); } catch {}
      }
    }
  }
  if (!signal.aborted) outgoing.end();
}

async function nextItem(iterator: AsyncIterator<Buffer>, signal: AbortSignal) {
  let onAbort: () => void = () => {};
  try {
    return await Promise.race([iterator.next(), new Promise<never>((resolveUnused, reject) => {
      onAbort = () => reject(new Error('request aborted'));
      if (signal.aborted) onAbort();
      else signal.addEventListener('abort', onAbort, { once: true });
    })]);
  } finally { signal.removeEventListener('abort', onAbort); }
}
