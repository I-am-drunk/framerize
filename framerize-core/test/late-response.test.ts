import assert from 'node:assert/strict';
import { createServer, request } from 'node:http';
import { once } from 'node:events';
import type { Socket } from 'node:net';
import { setImmediate } from 'node:timers/promises';
import test from 'node:test';
import { createInstance } from '../src/instance.js';
import type { InferenceBackend, InferenceResponse } from '../src/backend.js';
import { authorization, backend, deferred, exchange, token } from './http-fixtures.js';

const providerURL = new URL('../../../framerize-server/dist/provider.js', import.meta.url);
const { createProvider } = await import(providerURL.href) as {
  createProvider(options: object): InferenceBackend;
};

test('disposes a late provider socket before backend shutdown', { timeout: 4000 }, async (context) => {
  const sockets = new Set<Socket>();
  const offered = deferred<void>();
  const release = deferred<void>();
  const handled = deferred<void>();
  const upstream = createServer((request, response) => {
    request.resume();
    response.writeHead(200, { 'content-type': 'text/event-stream' });
    response.flushHeaders();
  });
  upstream.on('connection', (socket) => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
  });
  context.after(async () => {
    for (const socket of sockets) socket.destroy();
    await new Promise<void>((resolve) => upstream.close(() => resolve()));
  });
  upstream.listen(0, '127.0.0.1');
  await once(upstream, 'listening');
  const address = upstream.address();
  assert.ok(address && typeof address !== 'string');
  const provider = createProvider({ upstream: `http://127.0.0.1:${address.port}` });
  let closes = 0;
  const instance = createInstance({ token, headersTimeoutMs: 2000,
    onRequest() { handled.resolve(); }, backend: backend({
      async forward(input) {
        const result = await provider.forward({ ...input, signal: new AbortController().signal });
        offered.resolve();
        await release.promise;
        return result;
      },
      async close() { closes += 1; await provider.close(); },
    }) });
  context.after(async () => { release.resolve(); await instance.close(); });
  const { port } = await instance.listen();
  const client = request({ host: '127.0.0.1', port, path: '/v1/responses',
    method: 'POST', headers: authorization, agent: false });
  client.on('error', () => {});
  context.after(() => { client.destroy(); });
  client.end('{}');
  await offered.promise;
  assert.equal(sockets.size, 1);
  client.destroy();
  await handled.promise;
  const disconnected = once([...sockets][0]!, 'close', { signal: context.signal });
  release.resolve();
  await disconnected;
  assert.equal(sockets.size, 0);
  assert.equal(closes, 0);
});

test('closes a body rejected before streaming without closing successful bodies twice', async () => {
  const variants: { status: number; headers: Record<string, string>; expected: number }[] = [
    { status: 200, headers: {}, expected: 200 },
    { status: 199, headers: {}, expected: 502 },
    { status: 200, headers: { 'x-request-id': '\u2603' }, expected: 502 },
  ];
  for (const variant of variants) {
    let reads = 0;
    let returns = 0;
    const instance = createInstance({ token, backend: backend({ async forward() {
      return { ...variant, body: { [Symbol.asyncIterator]() { return {
        async next() { reads += 1; return { done: true as const, value: undefined }; },
        async return() { returns += 1; return { done: true as const, value: undefined }; },
      }; } } };
    } }) });

    try {
      const { port } = await instance.listen();
      const reply = await exchange(port, '/v1/responses', { method: 'POST', headers: authorization });
      assert.equal(reply.status, variant.expected);
      assert.equal(reads, variant.expected === 200 ? 1 : 0);
      assert.equal(returns, variant.expected === 200 ? 0 : 1);
    } finally { await instance.close(); }
  }
});

for (const mode of ['timeout', 'disconnect']) {
  test(`releases a response arriving after ${mode} without reading it`, { timeout: 3000 }, async () => {
    const pending = deferred<InferenceResponse>();
    const entered = deferred<void>();
    const finished = deferred<void>();
    let reads = 0;
    let returns = 0;
    const instance = createInstance({ token, inferenceTimeoutMs: 30,
      onRequest() { finished.resolve(); }, backend: backend({
        async forward() { entered.resolve(); return pending.promise; },
      }) });
    try {
      const { port } = await instance.listen();
      const completed = deferred<number | undefined>();
      const client = request({ host: '127.0.0.1', port, path: '/v1/responses',
        method: 'POST', headers: authorization, agent: false }, (response) => {
        response.resume();
        response.once('end', () => completed.resolve(response.statusCode));
      });
      client.on('error', () => completed.resolve(undefined));
      client.end('{}');
      await entered.promise;
      if (mode === 'disconnect') client.destroy();
      const status = await completed.promise;
      if (mode === 'timeout') assert.equal(status, 504);
      await finished.promise;
      pending.resolve({ status: 200, headers: {}, body: {
        [Symbol.asyncIterator]() { return {
          async next() { reads += 1; return { done: true as const, value: undefined }; },
          async return() { returns += 1; return { done: true as const, value: undefined }; },
        }; },
      } });
      await setImmediate();
      await setImmediate();
      assert.equal(reads, 0);
      assert.equal(returns, 1);
    } finally { await instance.close(); }
  });
}
