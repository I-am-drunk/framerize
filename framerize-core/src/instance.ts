import { createServer, type IncomingMessage, type ServerResponse } from 'node:http';
import { randomUUID } from 'node:crypto';
import type { AddressInfo, Socket } from 'node:net';
import type { ReadinessReason } from './readiness.js';
import type { InferenceResponse } from './backend.js';
import { HttpError, publicError } from './http-errors.js';
import { bodyIterator, bounded, finish, readBody, streamBody } from './http-io.js';
import { authenticated, capabilities, hashToken, requestHeaders, responseHeaders,
  routes, selectedHeaders, type Route } from './http-policy.js';
import { instanceOptions, type InstanceOptions, type RequestEvent } from './instance-options.js';

export interface FramerizeInstance {
  readonly server: ReturnType<typeof createServer>;
  listen(): Promise<AddressInfo>;
  close(): Promise<void>;
  readonly closing: boolean;
}

function json(response: ServerResponse, status: number, value: unknown): void {
  const body = JSON.stringify(value);
  response.writeHead(status, { 'content-type': 'application/json; charset=utf-8',
    'content-length': Buffer.byteLength(body), 'cache-control': 'no-store' });
  response.end(body);
}

function unavailable(reason: unknown): ReadinessReason {
  return reason === 'configuration_unavailable' || reason === 'credentials_unavailable'
    ? reason : 'upstream_unavailable';
}

function discardResponse(result: InferenceResponse): void {
  try { finish(bodyIterator(result.body)); } catch {}
}

export function createInstance(options: InstanceOptions): FramerizeInstance {
  const config = instanceOptions(options);
  const tokenHash = config.token === null ? null : hashToken(config.token);
  const active = new Set<AbortController>();
  const sockets = new Set<Socket>();
  let closing = false;
  let listening: Promise<AddressInfo> | undefined;
  let closed: Promise<void> | undefined;
  const server = createServer({ requestTimeout: 0, maxHeaderSize: 16_384,
    headersTimeout: config.requestHeadersTimeoutMs,
    connectionsCheckingInterval: Math.min(config.requestHeadersTimeoutMs, 1000),
  });
  server.maxHeadersCount = 0;
  server.on('connection', (socket) => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
    if (closing) socket.destroy();
  });
  server.on('request', (request, response) => { void handle(request, response); });
  function listen(): Promise<AddressInfo> {
    if (closing) return Promise.reject(new HttpError('server_closing'));
    listening ??= new Promise((resolve, reject) => {
      const cleanup = () => { server.off('error', failed); server.off('listening', ready); };
      const failed = () => { cleanup(); reject(new Error('Instance listen failed')); };
      const ready = () => { cleanup(); resolve(server.address() as AddressInfo); };
      server.once('error', failed);
      server.once('listening', ready);
      try { server.listen(config.port, config.host); } catch { failed(); }
    });
    return listening;
  }
  function close(): Promise<void> {
    if (closed) return closed;
    closing = true;
    closed = Promise.resolve().then(async () => {
      for (const controller of active) controller.abort(new HttpError('server_closing'));
      for (const socket of sockets) socket.destroy();
      await listening?.catch(() => {});
      await new Promise<void>((resolve) => server.close(() => resolve()));
      const controller = new AbortController();
      await bounded(() => Promise.resolve().then(() => config.backend.close()),
        controller, config.shutdownTimeoutMs, new HttpError('server_closing'))
        .catch(() => { throw new Error('Backend shutdown failed'); });
    });
    return closed;
  }
  return { server, listen, close, get closing() { return closing; } };
  async function handle(request: IncomingMessage, response: ServerResponse): Promise<void> {
    const requestId = randomUUID();
    const started = performance.now();
    const controller = new AbortController();
    const path = request.url?.split('?')[0] ?? '';
    const route = Object.hasOwn(routes, path) ? path as Route : 'unknown';
    let outcome: RequestEvent['outcome'] = 'completed';
    const disconnect = () => {
      if (!response.writableFinished) controller.abort(new HttpError('client_disconnected'));
    };
    active.add(controller);
    request.once('aborted', disconnect);
    response.once('close', disconnect);
    response.once('error', disconnect);
    try {
      if (closing) throw new HttpError('server_closing');
      if (route === 'unknown') throw new HttpError('not_found');
      if (request.method !== routes[route]) {
        response.setHeader('allow', routes[route]);
        throw new HttpError('method_not_allowed');
      }
      if (route === '/health') { json(response, 200, { status: 'ok', apiVersion: 1 }); return; }
      if (!authenticated(request, tokenHash)) throw new HttpError('unauthorized');
      if (route === '/ready') {
        let reason: ReadinessReason | undefined;
        try {
          const result = await bounded(() => config.backend.ready({ signal: controller.signal }),
            controller, config.headersTimeoutMs, new HttpError('headers_timeout'));
          if (result !== true) reason = unavailable(result?.reason);
        } catch (error) {
          try { reason = unavailable((error as { code?: unknown } | null)?.code); }
          catch { reason = 'upstream_unavailable'; }
        }
        if (response.destroyed) return;
        json(response, reason ? 503 : 200, { apiVersion: 1,
          status: reason ? 'unavailable' : 'ready', capabilities, ...(reason ? { reason } : {}) });
        return;
      }
      const bytes = await readBody(request, controller, config.maxBodyBytes, config.idleTimeoutMs);
      const result = await bounded(() => config.backend.forward({ path: route,
        headers: selectedHeaders(request.headers, requestHeaders),
        body: (async function* () { yield bytes; }()), signal: controller.signal,
      }), controller, config.inferenceTimeoutMs, new HttpError('headers_timeout'), discardResponse);
      try {
        controller.signal.throwIfAborted();
        if (!Number.isInteger(result.status) || result.status < 200 || result.status > 599) {
          throw new HttpError('upstream_error');
        }
        response.writeHead(result.status, selectedHeaders(result.headers, responseHeaders));
        response.flushHeaders();
      } catch (error) {
        discardResponse(result);
        throw error;
      }
      await streamBody(response, result.body, controller, config.idleTimeoutMs);
      response.end();
    } catch (error) {
      outcome = controller.signal.aborted
        && (controller.signal.reason as HttpError).code === 'client_disconnected' ? 'cancelled' : 'failed';
      const failure = publicError(error);
      if (response.headersSent || response.destroyed) response.destroy();
      else {
        response.shouldKeepAlive = false;
        if (failure.code === 'unauthorized') response.setHeader('www-authenticate', 'Bearer');
        json(response, failure.status, { error: {
          type: failure.code === 'unauthorized' ? 'authentication_error' : 'invalid_request_error',
          code: failure.code === 'request_timeout' ? 'idle_timeout' : failure.code,
          message: failure.message,
        } });
      }
      controller.abort(new HttpError('client_disconnected'));
    } finally {
      active.delete(controller);
      request.off('aborted', disconnect);
      response.off('close', disconnect);
      response.off('error', disconnect);
      try { void Promise.resolve(config.onRequest?.({ requestId, route, status: response.statusCode, outcome,
        durationMs: Math.round(performance.now() - started) })).catch(() => {}); } catch {}
    }
  }
}
