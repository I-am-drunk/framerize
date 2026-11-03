import assert from 'node:assert/strict';
import { once } from 'node:events';
import net from 'node:net';
import { createServer } from '../../dist/server.js';
import { loadConfig } from '../../dist/config.js';

export const TOKEN = 'acceptance-fixture-token';
export const JSON_BODY = Buffer.from('{"model":"fixture","input":"hello"}');
export const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

export function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

export async function waitFor(check, message = 'condition', timeoutMs = 2_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = await check();
    if (value) return value;
    await delay(5);
  }
  assert.fail(`Timed out waiting for ${message}`);
}

export function controlledStream(signal) {
  const queue = [];
  let waiting;
  let ended = false;
  let failure;
  const wake = () => { waiting?.(); waiting = undefined; };
  const abort = () => { failure = signal.reason || new Error('aborted'); wake(); };
  if (signal) {
    if (signal.aborted) abort();
    else signal.addEventListener('abort', abort, { once: true });
  }
  return {
    write(chunk) { queue.push(Buffer.from(chunk)); wake(); },
    end() { ended = true; wake(); },
    fail(error) { failure = error; wake(); },
    async *[Symbol.asyncIterator]() {
      while (queue.length || !ended) {
        if (failure) throw failure;
        if (!queue.length) await new Promise((resolve) => { waiting = resolve; });
        else yield queue.shift();
      }
    },
  };
}

export class FixtureProvider {
  constructor() {
    this.calls = [];
    this.readiness = true;
    this.forwardImpl = null;
    this.closeCalls = 0;
  }

  async ready() { return this.readiness; }
  async close() { this.closeCalls += 1; }

  async forward(call) {
    call.body = Buffer.isBuffer(call.body) ? call.body : await collect(call.body);
    this.calls.push(call);
    if (this.forwardImpl) return this.forwardImpl(call);
    return {
      status: 200,
      headers: { 'content-type': 'application/json', 'x-request-id': 'fixture-response' },
      body: [Buffer.from('{"id":"fixture-response"}')],
    };
  }
}

export async function collect(iterable) {
  const chunks = [];
  for await (const chunk of iterable) chunks.push(Buffer.from(chunk));
  return Buffer.concat(chunks);
}

export async function startFixture(t, values = {}, Provider = FixtureProvider) {
  const provider = new Provider();
  const app = await createServer(loadConfig({
    host: '127.0.0.1', port: 0, upstream: '', serverToken: TOKEN,
    idleTimeoutMs: 500, headersTimeoutMs: 1_000,
    requestHeadersTimeoutMs: 1_000, shutdownTimeoutMs: 1_000,
    ...values,
  }), { provider });
  t.after(async () => { await app.close(); });
  await app.listen();
  const port = app.server.address().port;
  return { app, provider, port, url: `http://127.0.0.1:${port}` };
}

export function authorizedHeaders(extra = {}) {
  return { authorization: `Bearer ${TOKEN}`, 'content-type': 'application/json', ...extra };
}

export async function apiRequest(port, path, {
  method = 'GET', headers = {}, body,
} = {}) {
  const response = await fetch(`http://127.0.0.1:${port}${path}`, {
    method, headers, body,
  });
  return { response, body: Buffer.from(await response.arrayBuffer()) };
}

export async function openSocket(port) {
  const socket = net.connect(port, '127.0.0.1');
  socket.setNoDelay(true);
  socket.setTimeout(3_000, () => socket.destroy(new Error('acceptance socket timeout')));
  await once(socket, 'connect');
  return socket;
}

export function socketChunks(socket) {
  const chunks = [];
  const closed = deferred();
  const failed = deferred();
  socket.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
  socket.on('close', () => closed.resolve());
  socket.on('error', (error) => failed.resolve(error));
  return {
    chunks,
    text: () => Buffer.concat(chunks).toString('latin1'),
    closed: closed.promise,
    failed: failed.promise,
    async waitFor(text) {
      await waitFor(() => Buffer.concat(chunks).includes(Buffer.from(text)), `socket data ${text}`);
    },
  };
}

export function rawPost(path = '/v1/responses', body = JSON_BODY) {
  return `POST ${path} HTTP/1.1\r\nHost: local\r\nAuthorization: Bearer ${TOKEN}\r\n`
    + `Content-Type: application/json\r\nContent-Length: ${body.length}\r\nConnection: close\r\n\r\n`;
}
