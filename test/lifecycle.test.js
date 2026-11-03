import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from '../src/server.js';
import { parseConfig } from '../src/config.js';
import net from 'node:net';
import { once } from 'node:events';
import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';

async function listen(t, config, fetchImpl) {
  const app = await createServer(parseConfig({ port: 0, upstream: 'http://fixture', ...config }), { fetchImpl });
  t.after(() => app.close());
  await app.listen();
  return { app, port: app.server.address().port };
}

test('idle upstream responses return a clean 502', async (t) => {
  const { port } = await listen(t, { idleTimeoutMs: 20 }, async () => new Response(
    new ReadableStream({ pull() { return new Promise(() => {}); } }),
  ));
  const response = await fetch(`http://127.0.0.1:${port}/v1/responses`, {
    method: 'POST', body: '{}',
  });
  assert.equal(response.status, 502);
  assert.match(await response.text(), /idle timeout/);
});

test('fetch failures return one sanitized 502 response', async (t) => {
  const { port } = await listen(t, {}, async () => { throw new Error('Bearer FAKE_SECRET'); });
  const response = await fetch(`http://127.0.0.1:${port}/v1/responses`, { method: 'POST', body: '{}' });
  assert.equal(response.status, 502);
  const body = await response.text();
  assert.ok(!body.includes('FAKE_SECRET'));
});

test('client disconnect aborts upstream once', async (t) => {
  let aborts = 0;
  let received;
  const aborted = new Promise((resolve) => { received = resolve; });
  const { port } = await listen(t, {}, async (url, options) => {
    options.signal.addEventListener('abort', () => { aborts += 1; received(); }, { once: true });
    return new Response(new ReadableStream({ pull() { return new Promise(() => {}); } }));
  });
  const controller = new AbortController();
  const pending = fetch(`http://127.0.0.1:${port}/v1/responses`, {
    method: 'POST', body: '{}', signal: controller.signal,
  });
  await new Promise((resolve) => setTimeout(resolve, 30));
  controller.abort();
  await assert.rejects(pending);
  await aborted;
  assert.equal(aborts, 1);
});

test('client disconnect logs are informational', async (t) => {
  const logs = [];
  const logger = {
    info: (event, fields) => logs.push({ level: 'info', event, fields }),
    error: (event, fields) => logs.push({ level: 'error', event, fields }),
  };
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-lifecycle-'));
  const app = await createServer(parseConfig({ port: 0, upstream: 'http://upstream.test' }), {
    logger, fetchImpl: async () => new Response(new ReadableStream({
      pull() { return new Promise(() => {}); },
    })),
  });
  t.after(() => app.close());
  await app.listen();
  const controller = new AbortController();
  const pending = fetch(`http://127.0.0.1:${app.server.address().port}/v1/responses`, {
    method: 'POST', body: '{}', signal: controller.signal,
  });
  await new Promise((resolve) => setTimeout(resolve, 30));
  controller.abort();
  await assert.rejects(pending);
  await new Promise((resolve) => setTimeout(resolve, 30));
  const failures = logs.filter(({ event }) => event === 'request.failed');
  assert.equal(failures.length, 1);
  assert.equal(failures[0].level, 'info');
  assert.equal(failures[0].fields.error.message, 'client disconnected');
});

test('shutdown aborts active upstream work and releases the port', async (t) => {
  let aborted;
  const cancellation = new Promise((resolve) => { aborted = resolve; });
  const { app, port } = await listen(t, {}, async (url, options) => {
    options.signal.addEventListener('abort', aborted, { once: true });
    return new Response(new ReadableStream({ pull() { return new Promise(() => {}); } }));
  });
  const pending = fetch(`http://127.0.0.1:${port}/v1/responses`, { method: 'POST', body: '{}' });
  await new Promise((resolve) => setTimeout(resolve, 30));
  await app.close();
  await assert.rejects(pending);
  await cancellation;
  assert.equal(app.server.listening, false);
});

test('request deadline releases downstream backpressure', async (t) => {
  const chunk = Buffer.alloc(65536, 1);
  const { app, port } = await listen(t, { requestTimeoutMs: 100 }, async () => new Response(
    ReadableStream.from(Array.from({ length: 2000 }, () => chunk)),
  ));
  const socket = net.connect(port, '127.0.0.1');
  socket.on('error', () => {});
  socket.on('data', () => {});
  socket.write('POST /v1/responses HTTP/1.1\r\nHost: local\r\nContent-Length: 2\r\n\r\n{}');
  const started = Date.now();
  await once(socket, 'close');
  assert.ok(Date.now() - started < 2000);
  assert.equal(app.server.listening, true);
});




