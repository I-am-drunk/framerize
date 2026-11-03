import http from 'node:http';
import { randomUUID } from 'node:crypto';
import { loadAccounts } from './config.js';
import { openUpstream, createDeadline } from './upstream.js';
import { idleTimeout, encodeSse, parseSse } from './sse.js';
import { sanitizeError } from './logger.js';
import { validateChat, responseEventError } from './openai.js';

function sendJson(response, status, value) {
  const body = JSON.stringify(value);
  response.writeHead(status, {
    'content-type': 'application/json; charset=utf-8',
    'content-length': Buffer.byteLength(body),
  });
  response.end(body);
}

async function* validateResponses(chunks) {
  yield* parseSse(chunks, (event) => {
    if (event.data === '[DONE]') return;
    const failure = responseEventError(JSON.parse(event.data));
    if (failure) throw failure;
  }, true);
}

function waitForDrain(response, signal) {
  return new Promise((resolve, reject) => {
    const done = () => { cleanup(); resolve(); };
    const failed = () => { cleanup(); reject(new Error('client disconnected')); };
    const aborted = () => { cleanup(); reject(signal.reason); };
    const cleanup = () => {
      response.off('drain', done);
      response.off('close', failed);
      signal?.removeEventListener('abort', aborted);
    };
    response.once('drain', done);
    response.once('close', failed);
    if (signal?.aborted) aborted();
    else signal?.addEventListener('abort', aborted, { once: true });
  });
}

async function readBody(request, limit) {
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > limit) {
      const error = new Error('request body too large');
      error.status = 413;
      throw error;
    }
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

function selectAccount(accounts, request, body) {
  const requested = request.headers['x-framer-account'];
  if (requested) return accounts.find((account) => account.id === requested);
  if (!accounts.length) return undefined;
  let model = '';
  try { model = JSON.parse(body.toString('utf8')).model || ''; } catch { model = ''; }
  return accounts.find((account) => account.model === model) || accounts[0];
}

export async function createServer(config, { logger, fetchImpl = fetch } = {}) {
  const accounts = await loadAccounts(config.accounts);
  const active = new Set();
  let closing = false;
  let closePromise;
  const server = http.createServer(async (request, response) => {
    const id = randomUUID();
    const started = performance.now();
    const controller = new AbortController();
    active.add(controller);
    const disconnect = () => controller.abort(new Error('client disconnected'));
    response.on('close', () => {
      active.delete(controller);
      if (!response.writableEnded) disconnect();
    });
    try {
      const url = new URL(request.url, 'http://local');
      if (request.method === 'GET' && url.pathname === '/health') {
        sendJson(response, 200, { ok: true, accounts: accounts.length });
        return;
      }
      if (request.method === 'GET' && url.pathname === '/v1/accounts') {
        sendJson(response, 200, {
          accounts: accounts.map(({ id, model, enabled, authorizationEnv, authorizationFile }) => ({
            id, model, enabled, credential: authorizationEnv ? 'environment'
              : authorizationFile ? 'file' : 'upstream',
          })),
        });
        return;
      }
      if (request.method !== 'POST' || !['/v1/responses', '/v1/chat/completions'].includes(url.pathname)) {
        sendJson(response, 404, { error: { message: 'not found', type: 'not_found' } });
        return;
      }
      let body = await readBody(request, config.maxBodyBytes);
      const account = selectAccount(accounts, request, body);
      if (request.headers['x-framer-account'] && !account) {
        sendJson(response, 404, { error: { message: 'unknown account', type: 'not_found' } });
        return;
      }
      const convertChat = account?.provider === 'framer'
        && url.pathname === '/v1/chat/completions';
      if (convertChat) {
        const payload = JSON.parse(body.toString('utf8'));
        validateChat(payload);
      }
      const upstream = await openUpstream({
        config, request, body, account, signal: controller.signal, fetchImpl,
      });
      const responseHeaders = {};
      for (const name of ['content-type', 'cache-control', 'x-request-id', 'retry-after']) {
        const value = upstream.response.headers.get(name);
        if (value) responseHeaders[name] = value;
      }
      const responseDeadline = createDeadline(controller.signal, config.requestTimeoutMs, disconnect);
      try {
        if (!upstream.response.ok) {
          response.writeHead(upstream.response.status, responseHeaders);
        }
        if (upstream.response.ok) response.writeHead(upstream.response.status, responseHeaders);
        if (upstream.response.body) {
          let chunks = idleTimeout(upstream.response.body, config.idleTimeoutMs, responseDeadline.signal);
          if (!convertChat && upstream.response.ok
              && responseHeaders['content-type']?.includes('application/json')) {
            chunks = validateResponses(chunks);
          }
          for await (const value of chunks) {
            if (!response.write(value)) await waitForDrain(response, responseDeadline.signal);
          }
        }
        response.end();
      } catch (error) {
        if (response.headersSent) response.destroy(error);
        throw error;
      } finally {
        upstream.deadline.clear();
        responseDeadline.clear();
      }
    } catch (error) {
      if (!response.headersSent) {
        const message = sanitizeError(error);
        sendJson(response, error.status || 502, {
          error: { message, type: error.status ? 'invalid_request' : 'upstream_error' },
        });
      } else if (!response.writableEnded) response.destroy(error);
      const log = error.message === 'client disconnected' ? logger?.info : logger?.error;
      log?.call(logger, 'request.failed', { requestId: id, error });
    } finally {
      logger?.info('request.completed', {
        requestId: id,
        method: request.method,
        path: new URL(request.url, 'http://local').pathname,
        status: response.statusCode,
        durationMs: Math.round(performance.now() - started),
      });
    }
  });
  server.keepAliveTimeout = 5_000;
  server.headersTimeout = 15_000;
  server.requestTimeout = 0;
  const trackSocket = (socket) => {
    active.add(socket);
    socket.on('close', () => active.delete(socket));
  };
  server.on('connection', trackSocket);
  return {
    server,
    async listen() {
      await new Promise((resolve, reject) => {
        server.once('error', reject);
        server.listen(config.port, config.host, resolve);
      });
      return server.address();
    },
    close() {
      if (!server.listening) return Promise.resolve();
      closePromise ??= (async () => {
        closing = true;
        for (const item of active) {
          if (item instanceof AbortController) item.abort(new Error('server shutting down'));
          else item.destroy();
        }
        await new Promise((resolve, reject) => {
          server.close((error) => error ? reject(error) : resolve());
        });
      })();
      return closePromise;
    },
  };
}
