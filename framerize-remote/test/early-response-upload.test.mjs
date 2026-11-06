import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createConnection } from 'node:net';
import { once, getEventListeners } from 'node:events';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/remoteBackend.ts', import.meta.url))],
  alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) },
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const bytes = compiled.outputFiles[0].contents;
const sourceDigest = createHash('sha256').update(bytes).digest('hex');
const { createRemoteBackend } = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
const ready = { apiVersion: 1, status: 'ready',
  capabilities: { responses: true, chatCompletions: true, streaming: true } };
const reply = Buffer.from('{"error":{"code":"payload_too_large"}}');
const deferred = () => {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
};
async function bounded(pending, message, milliseconds = 3000) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(message)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function refused(port) {
  const socket = createConnection({ host: '127.0.0.1', port });
  const closed = new Promise(resolve => socket.once('close', resolve));
  try {
    const error = await bounded(new Promise((resolve, reject) => {
      socket.once('error', resolve);
      socket.once('connect', () => reject(new Error('Fixture port is still open')));
    }), 'Port refusal check timed out');
    assert.equal(error.code, 'ECONNREFUSED');
  } finally { socket.destroy(); await bounded(closed, 'Port check socket did not close'); }
}

for (const path of ['/v1/responses', '/openai/v1/responses', '/anthropic/v1/messages']) {
  test(`early 413 releases a stalled upload without truncating ${path}`, { timeout: 10_000 }, async context => {
    const sockets = new Set();
    const started = deferred();
    const returned = deferred();
    const pendingItem = deferred();
    const controller = new AbortController();
    let pulls = 0;
    let returns = 0;
    let heldResponse;
    let backend;
    let operation;
    const requests = [];
    const requestChunks = [];
    const body = { [Symbol.asyncIterator]() { return this; },
      next() { return ++pulls === 1 ? Promise.resolve({ done: false, value: Buffer.from('{') }) : pendingItem.promise; },
      return() { returns += 1; pendingItem.resolve({ done: true }); returned.resolve(); return Promise.resolve({ done: true }); },
    };
    const fixture = createServer((incoming, outgoing) => {
      requests.push(incoming.url);
      incoming.on('error', () => {});
      outgoing.on('error', () => {});
      if (incoming.url === '/ready') {
        outgoing.setHeader('content-type', 'application/json');
        outgoing.end(JSON.stringify(ready));
        return;
      }
      incoming.on('data', chunk => requestChunks.push(chunk));
      incoming.once('data', () => {
        heldResponse = outgoing;
        outgoing.writeHead(413, { 'content-type': 'application/json' });
        outgoing.write(reply.subarray(0, 1), () => started.resolve());
      });
    });
    fixture.on('connection', socket => {
      sockets.add(socket);
      socket.once('close', () => sockets.delete(socket));
    });
    const cleanup = async () => {
      controller.abort();
      backend?.close();
      pendingItem.resolve({ done: true });
      for (const socket of sockets) socket.destroy();
      if (fixture.listening) await bounded(new Promise(resolve => fixture.close(resolve)), 'Fixture did not close');
      if (operation) await bounded(operation, 'Forwarding did not settle during cleanup');
    };
    context.after(cleanup);
    fixture.listen(0, '127.0.0.1');
    await bounded(once(fixture, 'listening'), 'Fixture did not listen');
    const port = fixture.address().port;
    backend = createRemoteBackend({ endpoint: { origin: `http://127.0.0.1:${port}`, loopback: true },
      serverToken: null, headersTimeoutMs: 5000, inferenceTimeoutMs: 5000,
      idleTimeoutMs: 5000, readinessBodyMaxBytes: 65536, readinessWholeMs: 5000 });
    operation = backend.forward({ path, body, headers: { 'content-type': 'application/json' },
      signal: controller.signal }).then(response => ({ response }), error => ({ error }));
    await bounded(Promise.race([started.promise, operation.then(result => {
      if (result.error) throw result.error;
      return started.promise;
    })]), 'The peer did not receive the first upload chunk');
    const earlyRelease = await bounded(returned.promise.then(() => true),
      'Upload iterator survived the early 413', 1000).catch(() => false);
    const responseOpenAtObservation = !heldResponse.writableEnded && !heldResponse.destroyed;
    heldResponse.end(reply.subarray(1));
    const result = await bounded(operation, 'The early response did not arrive');
    const chunks = [];
    if (result.response) {
      await bounded((async () => {
        for await (const chunk of result.response.body) chunks.push(chunk);
      })(), 'The early response body did not finish');
    }
    await bounded(returned.promise, 'Upload iterator did not return after response disposal');
    const abortListeners = getEventListeners(controller.signal, 'abort').length;
    await cleanup();
    await refused(port);
    context.diagnostic(JSON.stringify({ node: process.version, sourceDigest, path, earlyRelease,
      responseOpenAtObservation, pulls, returns, status: result.response?.status,
      requestBytes: Buffer.concat(requestChunks).length, responseBytes: Buffer.concat(chunks).length,
      abortListeners, fixturePortRefused: true }));
    assert.equal(result.error, undefined);
    assert.equal(result.response.status, 413);
    assert.deepEqual(Buffer.concat(chunks), reply);
    assert.deepEqual(requests, path === '/v1/responses' ? ['/ready', path] : [path]);
    assert.equal(Buffer.concat(requestChunks).toString(), '{');
    assert.equal(responseOpenAtObservation, true);
    assert.equal(pulls, 2);
    assert.equal(returns, 1);
    assert.equal(abortListeners, 0);
    assert.equal(earlyRelease, true, 'the rejected upload must release before the held response completes');
  });
}

test('a completed ordinary upload reaches the peer unchanged', { timeout: 5000 }, async context => {
  const sockets = new Set();
  const received = [];
  let backend;
  const fixture = createServer((incoming, outgoing) => {
    incoming.on('data', chunk => received.push(chunk));
    incoming.once('end', () => {
      outgoing.writeHead(201, { 'content-type': 'application/json' });
      outgoing.end('{"ok":true}');
    });
  });
  fixture.on('connection', socket => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
  });
  const cleanup = async () => {
    backend?.close();
    for (const socket of sockets) socket.destroy();
    if (fixture.listening) await bounded(new Promise(resolve => fixture.close(resolve)), 'Control fixture did not close');
  };
  context.after(cleanup);
  fixture.listen(0, '127.0.0.1');
  await bounded(once(fixture, 'listening'), 'Control fixture did not listen');
  const port = fixture.address().port;
  backend = createRemoteBackend({ endpoint: { origin: `http://127.0.0.1:${port}`, loopback: true },
    serverToken: null, headersTimeoutMs: 2000, inferenceTimeoutMs: 2000,
    idleTimeoutMs: 2000, readinessBodyMaxBytes: 65536, readinessWholeMs: 2000 });
  const controller = new AbortController();
  const body = (async function* () { yield Buffer.from('{"model":'); yield Buffer.from('"synthetic"}'); })();
  const response = await bounded(backend.forward({ path: '/openai/v1/responses', body,
    signal: controller.signal, headers: {} }), 'Control forwarding did not settle');
  const chunks = [];
  for await (const chunk of response.body) chunks.push(chunk);
  await cleanup();
  await refused(port);
  context.diagnostic(JSON.stringify({ node: process.version, sourceDigest,
    status: response.status, fixturePortRefused: true }));
  assert.equal(response.status, 201);
  assert.equal(Buffer.concat(received).toString(), '{"model":"synthetic"}');
  assert.equal(Buffer.concat(chunks).toString(), '{"ok":true}');
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});
