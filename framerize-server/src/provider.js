import http from 'node:http';
import https from 'node:https';
import { open as openFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import { ConfigurationError, ProviderError } from './errors.js';

const ROUTES = new Set(['/v1/responses', '/v1/chat/completions']);
const REQUEST_HEADERS = ['accept', 'content-type', 'openai-beta'];
const RESPONSE_HEADERS = ['content-type', 'content-encoding', 'cache-control',
  'x-request-id', 'retry-after'];
const ERROR_BODY = Buffer.from(JSON.stringify({ error: {
  type: 'upstream_error', code: 'upstream_http_error', message: 'Upstream request failed',
} }));
const failure = (message, code = 'upstream_error', status = 502) =>
  new ProviderError(message, { code, status });
const cancelled = () => failure('Request cancelled', 'request_cancelled', 499);

function duration(value, fallback) {
  const result = value ?? fallback;
  if (!Number.isSafeInteger(result) || result < 1 || result > 2_147_483_647) {
    throw new ConfigurationError('Timeout must be within the supported timer range');
  }
  return result;
}

function endpoint(value) {
  let url;
  try { url = new URL(value); } catch { throw new ConfigurationError('Invalid upstream URL'); }
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password
      || url.search || url.hash) throw new ConfigurationError('Invalid upstream URL');
  return url;
}

function selectedHeaders(headers, names) {
  const entries = Object.entries(headers ?? {}).map(([name, value]) => [name.toLowerCase(), value]);
  const normalized = Object.fromEntries(entries);
  const connection = String(normalized.connection ?? '').toLowerCase().split(',').map((name) => name.trim());
  return Object.fromEntries(names.filter((name) => typeof normalized[name] === 'string'
    && !connection.includes(name)).map((name) => [name, normalized[name]]));
}

async function wait(pending, signal, milliseconds, code = 'idle_timeout') {
  let timer;
  let abort;
  const stopped = new Promise((resolve, reject) => {
    abort = () => reject(signal.reason);
    if (signal.aborted) abort();
    else signal.addEventListener('abort', abort, { once: true });
    if (milliseconds) timer = setTimeout(() => reject(failure(
      code === 'headers_timeout' ? 'Upstream headers timeout' : 'Upstream idle timeout',
      code, 504,
    )), milliseconds);
  });
  try { return await Promise.race([stopped, pending]); } finally {
    clearTimeout(timer);
    signal.removeEventListener('abort', abort);
  }
}

async function credential(config, signal) {
  try {
    let value;
    if (config.credential) value = await config.credential({ signal });
    else if (config.authorizationEnv) value = process.env[config.authorizationEnv];
    else if (config.authorizationFile) value = await fileCredential(config.authorizationFile, signal);
    else value = config.authorization;
    if (value === undefined && !config.credential && !config.authorizationEnv
        && !config.authorizationFile) return undefined;
    if (typeof value !== 'string' || !value.trim()) throw new Error();
    const result = /^Bearer /i.test(value) ? value : `Bearer ${value}`;
    http.validateHeaderValue('authorization', result);
    return result;
  } catch {
    throw failure('Upstream credentials unavailable', 'credentials_unavailable', 503);
  }
}

async function fileCredential(path, signal) {
  const file = await openFile(path, constants.O_RDONLY | constants.O_NONBLOCK);
  try {
    const info = await file.stat();
    if (!info.isFile() || (process.platform !== 'win32' && (info.mode & 0o077))) throw new Error();
    const buffer = Buffer.alloc(16_385);
    let length = 0;
    while (length < buffer.length) {
      signal.throwIfAborted();
      const { bytesRead } = await file.read(buffer, length, buffer.length - length, null);
      if (!bytesRead) return buffer.subarray(0, length).toString('utf8').trim();
      length += bytesRead;
    }
    throw new Error();
  } finally { await file.close(); }
}

function exchange(active, parent) {
  const controller = new AbortController();
  const upload = new AbortController();
  const state = { controller, upload, response: null, request: null };
  const abort = () => state.dispose(cancelled());
  state.dispose = (error = cancelled()) => {
    controller.abort(error);
    upload.abort(error);
    state.response?.destroy();
    state.request?.destroy();
    parent?.removeEventListener('abort', abort);
    active.delete(state);
  };
  active.add(state);
  if (parent?.aborted) abort();
  else parent?.addEventListener('abort', abort, { once: true });
  return state;
}

async function drain(request, signal, milliseconds) {
  let drained;
  let closed;
  let failed;
  const pending = new Promise((resolve, reject) => {
    drained = resolve;
    closed = () => reject(failure('Upstream request closed'));
    failed = () => reject(failure('Upstream request failed'));
    request.once('drain', drained).once('close', closed).once('error', failed);
    if (request.destroyed) closed();
  });
  try { await wait(pending, signal, milliseconds); } finally {
    request.off('drain', drained).off('close', closed).off('error', failed);
  }
}

async function sendBody(state, body, idleTimeoutMs) {
  let iterator;
  let complete = false;
  try {
    iterator = body[Symbol.asyncIterator]();
    while (!state.upload.signal.aborted) {
      const next = await wait(Promise.resolve().then(() => iterator.next()).catch(() => {
        throw failure('Request body failed', 'request_body_error', 400);
      }), state.upload.signal, idleTimeoutMs);
      if (next.done) { complete = true; state.request.end(); return; }
      if (!Buffer.isBuffer(next.value)) {
        throw failure('Request body must yield Buffers', 'invalid_request_body', 400);
      }
      if (!state.request.write(next.value)) {
        await drain(state.request, state.upload.signal, idleTimeoutMs);
      }
    }
  } catch (error) {
    if (!state.upload.signal.aborted) state.dispose(error instanceof ProviderError
      ? error : failure('Request body failed', 'request_body_error', 400));
  } finally {
    if (!complete) Promise.resolve().then(() => iterator?.return?.()).catch(() => {});
  }
}

function startHeaderTimer(state, milliseconds) {
  return setTimeout(() => state.dispose(failure(
    'Upstream headers timeout', 'headers_timeout', 504,
  )), milliseconds);
}

async function open(state, url, method, headers, body, config) {
  const { signal } = state.controller;
  const auth = await wait(credential(config, signal), signal,
    config.headersTimeoutMs, 'headers_timeout');
  signal.throwIfAborted();
  const transport = url.protocol === 'https:' ? https : http;
  const request = transport.request(url, {
    method, agent: false, headers: { ...headers, ...(auth ? { authorization: auth } : {}) },
  });
  state.request = request;
  let connectTimer = startHeaderTimer(state, config.headersTimeoutMs);
  let headersTimer;
  let received = false;
  const clear = () => { clearTimeout(connectTimer); clearTimeout(headersTimer); };
  signal.addEventListener('abort', clear, { once: true });
  request.once('socket', (socket) => {
    const connected = () => { clearTimeout(connectTimer); connectTimer = undefined; };
    if (socket.connecting) socket.once(url.protocol === 'https:' ? 'secureConnect' : 'connect', connected);
    else connected();
  });
  request.once('finish', () => {
    if (!received && !signal.aborted) headersTimer = startHeaderTimer(state, config.headersTimeoutMs);
  });
  request.on('error', () => {
    if (!received) state.dispose(failure('Upstream request failed'));
  });
  request.once('upgrade', (response, socket) => {
    socket.destroy();
    state.dispose(failure('Unsupported upstream response'));
  });
  request.once('close', () => {
    if (!received) state.dispose(failure('Upstream request closed'));
  });
  const responsePending = new Promise((resolve) => request.once('response', (response) => {
    received = true;
    clear();
    state.response = response;
    response.on('error', () => state.dispose(failure('Upstream response failed')));
    response.once('aborted', () => state.dispose(failure('Upstream response failed')));
    state.upload.abort(cancelled());
    if (!request.writableEnded) request.end();
    resolve(response);
  }));
  if (body) void sendBody(state, body, config.idleTimeoutMs);
  else request.end();
  try {
    const response = await wait(responsePending, signal);
    if (response.statusCode < 200 || response.statusCode > 599) {
      throw failure('Unsupported upstream status');
    }
    return response;
  } finally {
    clear();
    signal.removeEventListener('abort', clear);
  }
}

function responseBody(state, idleTimeoutMs) {
  const iterator = (async function* () {
    try {
      const source = state.response[Symbol.asyncIterator]();
      while (true) {
        state.controller.signal.throwIfAborted();
        const next = await wait(source.next(), state.controller.signal, idleTimeoutMs);
        if (next.done) return;
        yield next.value;
      }
    } catch (error) {
      throw error instanceof ProviderError ? error : failure('Upstream response failed');
    } finally { state.dispose(); }
  })();
  return {
    next: () => iterator.next(),
    return() {
      state.dispose();
      return iterator.return();
    },
    throw() {
      state.dispose();
      return iterator.throw(cancelled());
    },
    [Symbol.asyncIterator]() { return this; },
  };
}

function discardError(state) {
  let bytes = 0;
  try {
    const reason = state.controller.signal.reason;
    if (['request_cancelled', 'provider_closed'].includes(reason?.code)) throw reason;
    while (bytes < 65_536 && state.response.readableLength) {
      const chunk = state.response.read(Math.min(65_536 - bytes, state.response.readableLength));
      if (!chunk) break;
      bytes += chunk.length;
    }
  } finally { state.dispose(); }
}

export function createProvider(values = {}) {
  const base = endpoint(values.upstream);
  const config = { ...values,
    headersTimeoutMs: duration(values.headersTimeoutMs, 15_000),
    idleTimeoutMs: duration(values.idleTimeoutMs, 60_000),
  };
  const active = new Set();
  let closed = false;
  let closePromise;
  const begin = (signal) => {
    if (closed) throw failure('Provider closed', 'provider_closed', 503);
    if (signal?.aborted) throw cancelled();
    return exchange(active, signal);
  };
  const destination = (path) => new URL(`${base.href.replace(/\/+$/, '')}${path}`);
  return {
    async forward({ path, body, headers, signal }) {
      if (!ROUTES.has(path)) throw failure('Unsupported provider route', 'not_found', 404);
      if (!body || typeof body[Symbol.asyncIterator] !== 'function') {
        throw failure('Request body must be an async iterable of Buffers', 'invalid_request_body', 400);
      }
      const state = begin(signal);
      try {
        const selected = selectedHeaders(headers, REQUEST_HEADERS);
        selected.accept ??= 'text/event-stream, application/json';
        selected['content-type'] ??= 'application/json';
        const response = await open(state, destination(path), 'POST', selected, body, config);
        const status = response.statusCode;
        if (status >= 200 && status < 300) return {
          status, headers: selectedHeaders(response.headers, RESPONSE_HEADERS),
          body: responseBody(state, config.idleTimeoutMs),
        };
        const safeHeaders = selectedHeaders(response.headers, ['retry-after', 'x-request-id']);
        discardError(state);
        return {
          status, headers: { ...safeHeaders, 'content-type': 'application/json',
            'content-length': String(ERROR_BODY.length) },
          body: (async function* () { yield Buffer.from(ERROR_BODY); })(),
        };
      } catch (error) {
        state.dispose();
        throw error instanceof ProviderError ? error : failure('Upstream request failed');
      }
    },
    async ready({ signal } = {}) {
      const state = begin(signal);
      try {
        const response = await open(state, destination('/v1/models'), 'GET', {}, null, config);
        return response.statusCode < 300 ? true : { ready: false, reason: 'upstream_unavailable' };
      } catch (error) {
        if (['request_cancelled', 'provider_closed'].includes(error.code)) throw error;
        return { ready: false, reason: error.code === 'credentials_unavailable'
          ? 'credentials_unavailable' : 'upstream_unavailable' };
      } finally { state.dispose(); }
    },
    close() {
      if (closePromise) return closePromise;
      closed = true;
      const sockets = [...active].map((state) => {
        const request = state.request;
        const pending = request && !request.closed
          ? new Promise((resolve) => request.once('close', resolve)) : Promise.resolve();
        state.dispose(failure('Provider closed', 'provider_closed', 503));
        return pending;
      });
      closePromise = Promise.all(sockets).then(() => {});
      return closePromise;
    },
  };
}
