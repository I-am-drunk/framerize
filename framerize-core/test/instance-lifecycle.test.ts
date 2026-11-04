import assert from 'node:assert/strict';
import { getEventListeners, once } from 'node:events';
import { request, type IncomingMessage } from 'node:http';
import { setTimeout as delay } from 'node:timers/promises';
import test from 'node:test';
import { createInstance } from '../src/instance.js';
import { authorization, backend, deferred, exchange, token, unavailable } from './http-fixtures.js';

function streamingRequest(port: number): Promise<IncomingMessage> {
  return new Promise((resolve, reject) => {
    const client = request({ host: '127.0.0.1', port, path: '/v1/responses',
      method: 'POST', agent: false, headers: authorization }, (response) => {
      response.on('error', () => {});
      resolve(response);
    });
    client.on('error', reject);
    client.setTimeout(3000, () => client.destroy(new Error('Stream fixture timed out')));
    client.end('{}');
  });
}

test('server shutdown aborts active work and closes exactly once', { timeout: 3000 }, async () => {
  const entered = deferred<AbortSignal>();
  let closes = 0;
  const instance = createInstance({ token, backend: backend({
    async forward({ signal }) { entered.resolve(signal); return new Promise(() => {}); },
    close() { closes += 1; },
  }) });
  const { port } = await instance.listen();
  const pending = assert.rejects(exchange(port, '/v1/responses',
    { method: 'POST', headers: authorization }));
  const signal = await entered.promise;
  await instance.close();
  await pending;
  assert.equal(signal.aborted, true);
  assert.equal(getEventListeners(signal, 'abort').length, 0);
  assert.equal(closes, 1);
  await unavailable(port);
});

test('client cancellation aborts the backend and releases its iterator', { timeout: 3000 }, async () => {
  const released = deferred<void>();
  let signal!: AbortSignal;
  let closes = 0;
  const instance = createInstance({ token, backend: backend({
    async forward(context) {
      signal = context.signal;
      return { status: 200, headers: {}, body: (async function* () {
        try {
          yield Buffer.from('first');
          await new Promise<void>((resolve) => signal.addEventListener('abort',
            () => resolve(), { once: true }));
        } finally { released.resolve(); }
      })() };
    },
    close() { closes += 1; },
  }) });
  const { port } = await instance.listen();
  try {
    const response = await streamingRequest(port);
    await once(response, 'data');
    response.destroy();
    await released.promise;
    assert.equal(signal.aborted, true);
    assert.equal(getEventListeners(signal, 'abort').length, 0);
  } finally { await instance.close(); }
  assert.equal(closes, 1);
  await unavailable(port);
});

test('idle stream aborts and calls iterator return', { timeout: 3000 }, async () => {
  let returned = 0;
  let signal!: AbortSignal;
  const instance = createInstance({ token, idleTimeoutMs: 40, backend: backend({
    async forward(context) {
      signal = context.signal;
      return { status: 200, headers: {}, body: { [Symbol.asyncIterator]() {
        return { next: () => new Promise(() => {}), async return() {
          returned += 1;
          return { done: true as const, value: undefined };
        } };
      } } };
    },
  }) });
  try {
    const { port } = await instance.listen();
    const response = await streamingRequest(port);
    await once(response, 'aborted');
    assert.equal(signal.aborted, true);
    assert.equal(returned, 1);
    assert.equal(getEventListeners(signal, 'abort').length, 0);
  } finally { await instance.close(); }
});

test('slow readers apply backpressure without buffering the whole response', { timeout: 5000 }, async () => {
  let produced = 0;
  const chunk = Buffer.alloc(2 * 1024 * 1024);
  const instance = createInstance({ token, idleTimeoutMs: 1000, backend: backend({
    async forward() {
      return { status: 200, headers: {}, body: (async function* () {
        for (let index = 0; index < 20; index += 1) { produced += 1; yield chunk; }
      })() };
    },
  }) });
  try {
    const { port } = await instance.listen();
    const response = await streamingRequest(port);
    response.pause();
    await delay(50);
    assert.ok(produced < 20, `produced ${produced} chunks before the client consumed them`);
    let bytes = 0;
    response.on('data', (value: Buffer) => { bytes += value.length; });
    const ended = once(response, 'end');
    response.resume();
    await ended;
    assert.equal(bytes, 20 * chunk.length);
    assert.equal(produced, 20);
  } finally { await instance.close(); }
});

test('a stalled or throwing backend cannot hold shutdown open', async () => {
  for (const close of [() => new Promise<void>(() => {}), () => { throw new Error('hidden-close'); }]) {
    const instance = createInstance({ token, shutdownTimeoutMs: 35, backend: backend({ close }) });
    const { port } = await instance.listen();
    await assert.rejects(instance.close(), { message: 'Backend shutdown failed' });
    await unavailable(port);
  }
});

test('healthy inference has no total deadline and preserves streamed bytes', async () => {
  const instance = createInstance({ token, inferenceTimeoutMs: 30, idleTimeoutMs: 200,
    backend: backend({ async forward() {
      return { status: 200, headers: { 'content-type': 'text/event-stream' },
        body: (async function* () {
          for (let index = 0; index < 5; index += 1) {
            await delay(40);
            yield Buffer.from(`data: ${index}\n\n`);
          }
        })() };
    } }) });
  try {
    const { port } = await instance.listen();
    const started = performance.now();
    const response = await exchange(port, '/v1/responses', { method: 'POST', headers: authorization });
    assert.equal(response.status, 200);
    assert.equal(response.body.toString(), Array.from({ length: 5 },
      (_, index) => `data: ${index}\n\n`).join(''));
    assert.ok(performance.now() - started >= 200);
  } finally { await instance.close(); }
});
