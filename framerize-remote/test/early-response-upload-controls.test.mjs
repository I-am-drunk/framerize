import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createConnection } from 'node:net';
import { once, getEventListeners } from 'node:events';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/remoteBackend.ts', import.meta.url))],
  alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) },
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
});
stop();
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const bytes = compiled.outputFiles[0].contents;
const { createRemoteBackend } = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
console.log(JSON.stringify({ runtime: process.version, bundle: hash(bytes),
  inputs: Object.keys(compiled.metafile.inputs).sort().map(path => ({ path, sha256: hash(readFileSync(path)) })) }));

async function within(promise, message, milliseconds = 2000) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolveUnused, reject) => {
      timer = setTimeout(() => reject(new Error(message)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

const first = Buffer.from('{"model":');
const last = Buffer.from('"synthetic"}');

async function fixture(context, status) {
  const sockets = new Set(), received = [];
  const firstRead = Promise.withResolvers(), uploadEnded = Promise.withResolvers();
  const returned = Promise.withResolvers(), pendingItem = Promise.withResolvers();
  const peerClosed = Promise.withResolvers();
  const controller = new AbortController();
  let pulls = 0, returns = 0, outgoing, backend, operation;
  const prefix = Buffer.from('event: synthetic\ndata: {"part":1}\n\n');
  const suffix = Buffer.from('event: synthetic\ndata: {"part":2}\n\n');
  const body = { [Symbol.asyncIterator]() { return this; },
    next() {
      pulls++;
      if (pulls === 1) return Promise.resolve({ value: first, done: false });
      if (pulls === 2) { firstRead.resolve(); return pendingItem.promise; }
      return Promise.resolve({ done: true });
    },
    return() {
      returns++;
      pendingItem.resolve({ done: true });
      returned.resolve();
      return Promise.resolve({ done: true });
    },
  };
  const server = createServer((request, response) => {
    outgoing = response;
    request.on('error', () => {});
    response.on('error', () => {});
    request.once('end', () => uploadEnded.resolve());
    request.on('data', chunk => received.push(chunk));
    request.once('data', () => { response.writeHead(status, { 'content-type': 'text/event-stream' }); response.write(prefix); });
  });
  server.on('connection', socket => {
    sockets.add(socket);
    socket.once('close', () => { sockets.delete(socket); peerClosed.resolve(); });
  });
  const cleanup = async () => {
    controller.abort();
    pendingItem.resolve({ done: true });
    await backend?.close();
    for (const socket of sockets) socket.destroy();
    if (server.listening) await within(new Promise(resolve => server.close(resolve)), 'Fixture close timed out');
    if (operation) await within(operation, 'Forward cleanup timed out');
  };
  context.after(cleanup);
  server.listen(0, '127.0.0.1');
  await within(once(server, 'listening'), 'Fixture listen timed out');
  const port = server.address().port;
  backend = createRemoteBackend({ endpoint: { origin: `http://127.0.0.1:${port}`, loopback: true },
    serverToken: null, headersTimeoutMs: 5000, inferenceTimeoutMs: 5000,
    idleTimeoutMs: 5000, readinessBodyMaxBytes: 65536, readinessWholeMs: 5000 });
  operation = backend.forward({ path: '/openai/v1/responses', body,
    headers: { 'content-type': 'application/json' }, signal: controller.signal })
    .then(response => ({ response }), error => ({ error }));
  const result = await within(operation, 'Response headers timed out');
  assert.equal(result.error, undefined);
  await within(firstRead.promise, 'Upload did not reach its pending pull');
  return { response: result.response, controller, received, prefix, suffix, port,
    returned: returned.promise, uploadEnded: uploadEnded.promise, peerClosed: peerClosed.promise,
    sendLast: () => pendingItem.resolve({ value: last, done: false }),
    finish: () => outgoing.end(suffix), open: () => !outgoing.writableEnded && !outgoing.destroyed,
    stats: () => ({ pulls, returns }), cleanup };
}

async function streamStart(state) {
  const iterator = state.response.body[Symbol.asyncIterator]();
  const chunks = [];
  while (Buffer.concat(chunks).length < state.prefix.length) {
    const item = await within(iterator.next(), 'First response event timed out');
    assert.equal(item.done, false);
    chunks.push(item.value);
  }
  assert.deepEqual(Buffer.concat(chunks), state.prefix);
  return { iterator, chunks };
}

async function streamEnd(stream) {
  for (;;) {
    const item = await within(stream.iterator.next(), 'Response completion timed out');
    if (item.done) return Buffer.concat(stream.chunks);
    stream.chunks.push(item.value);
  }
}

async function assertRefused(port) {
  const socket = createConnection({ host: '127.0.0.1', port });
  const closed = new Promise(resolve => socket.once('close', resolve));
  try {
    const result = await within(new Promise(resolve => {
      socket.once('error', error => resolve(error.code));
      socket.once('connect', () => resolve('connected'));
    }), 'Refusal check timed out');
    assert.equal(result, 'ECONNREFUSED');
  } finally { socket.destroy(); await within(closed, 'Refusal socket did not close'); }
}

test('an early successful stream still permits the complete upload', { timeout: 8000 }, async context => {
  const state = await fixture(context, 200);
  const stream = await streamStart(state);
  assert.deepEqual(state.stats(), { pulls: 2, returns: 0 });
  assert.equal(state.open(), true);
  state.sendLast();
  await within(state.uploadEnded, 'Successful upload did not finish');
  state.finish();
  const received = await streamEnd(stream);
  await within(state.peerClosed, 'Successful exchange socket stayed open');
  const listeners = getEventListeners(state.controller.signal, 'abort').length;
  await state.cleanup();
  await assertRefused(state.port);
  assert.equal(state.response.status, 200);
  assert.deepEqual(Buffer.concat(state.received), Buffer.concat([first, last]));
  assert.deepEqual(received, Buffer.concat([state.prefix, state.suffix]));
  assert.deepEqual(state.stats(), { pulls: 3, returns: 0 });
  assert.equal(listeners, 0);
  context.diagnostic('Early success preserves full upload and SSE; socket closed and port refused');
});

test('an early streaming rejection releases the upload before response completion', { timeout: 8000 }, async context => {
  const state = await fixture(context, 413);
  const stream = await streamStart(state);
  const releasedEarly = await within(state.returned, 'Upload remained pending', 500)
    .then(() => true, () => false);
  const openAtObservation = state.open();
  state.finish();
  const received = await streamEnd(stream);
  await within(state.returned, 'Upload was not released after response completion');
  await within(state.peerClosed, 'Rejected exchange socket stayed open');
  const listeners = getEventListeners(state.controller.signal, 'abort').length;
  await state.cleanup();
  await assertRefused(state.port);
  assert.equal(state.response.status, 413);
  assert.deepEqual(Buffer.concat(state.received), first);
  assert.deepEqual(received, Buffer.concat([state.prefix, state.suffix]));
  assert.equal(openAtObservation, true);
  assert.deepEqual(state.stats(), { pulls: 2, returns: 1 });
  assert.equal(listeners, 0);
  context.diagnostic(JSON.stringify({ releasedEarly, responsePreserved: true, fixturePortRefused: true }));
  assert.equal(releasedEarly, true, 'The rejected upload must release while the SSE response is still open');
});

test('caller cancellation closes an unfinished response and pending upload', { timeout: 8000 }, async context => {
  const state = await fixture(context, 200);
  const stream = await streamStart(state);
  const pending = stream.iterator.next().then(value => ({ value }), error => ({ error }));
  state.controller.abort(new Error('synthetic caller cancellation'));
  const result = await within(pending, 'Cancelled response did not settle');
  await within(state.returned, 'Cancelled upload was not returned');
  await within(state.peerClosed, 'Caller cancellation did not close the peer socket');
  const listeners = getEventListeners(state.controller.signal, 'abort').length;
  await state.cleanup();
  await assertRefused(state.port);
  assert.ok(result.error instanceof Error);
  assert.deepEqual(state.stats(), { pulls: 2, returns: 1 });
  assert.deepEqual(Buffer.concat(state.received), first);
  assert.equal(listeners, 0);
  context.diagnostic('Caller cancellation returned the upload and closed the peer before fixture cleanup');
});
