import http from 'node:http';
import { createHash, randomUUID, timingSafeEqual } from 'node:crypto';
import { loadConfig, readServerToken } from './config.js';
import { loadAccounts } from './accounts.js';
import { lifecycle } from './lifecycle.js';
import { appError, bounded, drain, finishIterator, readBody } from './transport.js';

const ROUTES = new Map([['/health', 'GET'], ['/ready', 'GET'],
  ['/v1/responses', 'POST'], ['/v1/chat/completions', 'POST']]);
const CAPABILITIES = { responses: true, chatCompletions: true, streaming: true };
const REASONS = new Set(['configuration_unavailable', 'credentials_unavailable', 'upstream_unavailable']);
const RESPONSE_HEADERS = ['content-type', 'content-encoding', 'cache-control', 'retry-after', 'x-request-id'];
const MESSAGES = {
  not_found: 'Not found', method_not_allowed: 'Method not allowed',
  unauthorized: 'Authentication required', invalid_json: 'Invalid JSON body',
  body_too_large: 'Request body too large', headers_timeout: 'Upstream headers timed out',
  idle_timeout: 'Request timed out', server_closing: 'Server is closing',
  provider_unavailable: 'Provider unavailable', provider_closed: 'Provider unavailable',
  credentials_unavailable: 'Provider unavailable', configuration_error: 'Provider unavailable',
  upstream_error: 'Upstream request failed', upstream_http_error: 'Upstream request failed',
};

function sendJson(response, status, value, headers = {}) {
  const body = JSON.stringify(value);
  response.writeHead(status, { ...headers, 'content-type': 'application/json; charset=utf-8',
    'content-length': Buffer.byteLength(body), 'cache-control': 'no-store' });
  response.end(body);
}

function authenticated(request, tokenHash, configured) {
  const authorization = request.headers.authorization;
  const count = request.rawHeaders.filter((value, index) => index % 2 === 0 && value.toLowerCase() === 'authorization').length;
  if (!configured || count !== 1 || typeof authorization !== 'string') return false;
  const match = /^Bearer ([A-Za-z0-9._~+/-]+=*)$/i.exec(authorization);
  return Boolean(match && timingSafeEqual(tokenHash, createHash('sha256').update(match[1]).digest()));
}

function safeHeaders(headers = {}) {
  const result = {};
  for (const [name, value] of Object.entries(headers)) {
    if (RESPONSE_HEADERS.includes(name.toLowerCase()) && typeof value === 'string'
      && !/[\r\n\0]/.test(value)) result[name.toLowerCase()] = value;
  }
  return result;
}

async function streamBody(response, body, controller, config) {
  const iterator = body?.[Symbol.asyncIterator]?.() ?? body?.[Symbol.iterator]?.();
  if (!iterator) throw appError(502, 'upstream_error', 'Upstream request failed');
  let completed = false;
  try {
    while (true) {
      const { value, done } = await bounded(() => iterator.next(), controller, config.idleTimeoutMs,
        appError(504, 'idle_timeout', 'Upstream body timed out'));
      if (done) { completed = true; break; }
      if (!Buffer.isBuffer(value)) throw appError(502, 'upstream_error', 'Upstream request failed');
      if (!response.write(value)) await drain(response, controller, config.idleTimeoutMs);
    }
  } finally {
    if (!completed) finishIterator(iterator);
  }
}

export async function createServer(values = {}, { provider, logger } = {}) {
  const config = loadConfig(values);
  const token = await readServerToken(config);
  const tokenHash = createHash('sha256').update(token).digest();
  await loadAccounts(config.accountsFile);
  const active = new Set();
  let app;
  const server = http.createServer({ requestTimeout: 0,
    headersTimeout: config.requestHeadersTimeoutMs,
    connectionsCheckingInterval: Math.min(config.requestHeadersTimeoutMs, 1000),
  }, (request, response) => { void handle(request, response); });
  async function handle(request, response) {
    const requestId = randomUUID();
    const started = performance.now();
    const controller = new AbortController();
    const path = request.url?.split('?')[0];
    const route = ROUTES.has(path) ? path : 'unknown';
    let outcome = 'completed';
    active.add(controller);
    const disconnect = () => {
      if (!response.writableFinished) controller.abort(appError(499, 'client_disconnected', 'Client disconnected'));
    };
    request.once('aborted', disconnect);
    response.once('close', disconnect);
    response.once('error', disconnect);
    try {
      if (app.closing) throw appError(503, 'server_closing', 'Server is closing');
      if (!ROUTES.has(path)) throw appError(404, 'not_found', 'Not found');
      if (request.method !== ROUTES.get(path)) {
        response.setHeader('allow', ROUTES.get(path));
        throw appError(405, 'method_not_allowed', 'Method not allowed');
      }
      if (path === '/health') { sendJson(response, 200, { status: 'ok', apiVersion: 1 }); return; }
      if (!authenticated(request, tokenHash, Boolean(token))) throw appError(401, 'unauthorized', 'Authentication required');
      const headerTimeout = appError(504, 'headers_timeout', 'Upstream headers timed out');
      if (path === '/ready') {
        let reason;
        if (!provider?.forward || !provider?.ready) reason = 'configuration_unavailable';
        else {
          try {
            const result = await bounded(() => provider.ready({ signal: controller.signal }),
              controller, config.headersTimeoutMs, headerTimeout);
            if (result !== true) reason = REASONS.has(result?.reason) ? result.reason : 'upstream_unavailable';
          } catch (error) { reason = REASONS.has(error?.code) ? error.code : 'upstream_unavailable'; }
        }
        sendJson(response, reason ? 503 : 200, { apiVersion: 1, status: reason ? 'unavailable' : 'ready',
          capabilities: CAPABILITIES, ...(reason ? { reason } : {}) });
        return;
      }
      if (!provider?.forward) throw appError(503, 'provider_unavailable', 'Provider unavailable');
      const bytes = await readBody(request, controller, config);
      const headers = {};
      for (const name of ['accept', 'content-type', 'openai-beta']) {
        if (typeof request.headers[name] === 'string') headers[name] = request.headers[name];
      }
      const result = await bounded(() => provider.forward({ path, headers,
        body: (async function* () { yield bytes; }()), signal: controller.signal,
      }), controller, config.headersTimeoutMs, headerTimeout);
      if (!Number.isInteger(result?.status) || result.status < 200 || result.status > 599) {
        throw appError(502, 'upstream_error', 'Upstream request failed');
      }
      response.writeHead(result.status, safeHeaders(result.headers));
      response.flushHeaders();
      await streamBody(response, result.body, controller, config);
      response.end();
    } catch (error) {
      outcome = controller.signal.reason?.code === 'client_disconnected' ? 'cancelled' : 'failed';
      if (response.headersSent || response.destroyed) response.destroy();
      else {
        const code = Object.hasOwn(MESSAGES, error?.code) ? error.code : 'upstream_error';
        const status = Number.isInteger(error?.status) && error.status >= 400 && error.status <= 599 ? error.status : 502;
        response.shouldKeepAlive = false;
        sendJson(response, status, { error: { type: code === 'unauthorized' ? 'authentication_error' : 'invalid_request_error',
          code, message: MESSAGES[code] } }, code === 'unauthorized' ? { 'www-authenticate': 'Bearer' } : {});
      }
      controller.abort(appError(499, 'request_cancelled', 'Request cancelled'));
    } finally {
      active.delete(controller);
      request.off('aborted', disconnect);
      try { logger?.info?.('request.finished', { requestId, route, status: response.statusCode,
        outcome, durationMs: Math.round(performance.now() - started) }); } catch {}
    }
  }
  app = lifecycle(server, config, provider, active);
  return app;
}
