import assert from 'node:assert/strict';
import test from 'node:test';
import http from 'node:http';
import net from 'node:net';
import { once } from 'node:events';
import { createProvider } from '../dist/provider.js';

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}
async function bounded(promise, label) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolveUnused, reject) => {
      timer = setTimeout(() => reject(new Error(`Timed out: ${label}`)), 2000);
    })]);
  } finally { clearTimeout(timer); }
}

async function refused(port) {
  const connection = net.connect({ host: '127.0.0.1', port });
  try {
    const error = await bounded(new Promise(resolve => {
      connection.once('error', resolve);
      connection.once('connect', () => resolve(new Error('Listener still open')));
    }), 'closed listener');
    assert.equal(error.code, 'ECONNREFUSED');
  } finally { connection.destroy(); }
}

for (const early of [false, true]) {
  test(`${early ? 'early 200' : 'complete upload'} preserves the whole request`,
    { timeout: 5000 }, async context => {
    const sockets = new Set();
    const ended = deferred();
    const release = deferred();
    const chunks = [];
    const prefix = Buffer.from('{"model":"synthetic","input":"');
    const suffix = Buffer.from('local only"}');
    const expected = Buffer.concat([prefix, suffix]);
    const responseText = 'data: {"synthetic":true}\n\ndata: [DONE]\n\n';
    let pulls = 0;
    let returns = 0;
    let provider;
    let cleaning;
    let server;
    const cleanup = () => cleaning ??= (async () => {
      release.resolve();
      const closes = [...sockets].map(socket => new Promise(resolve => socket.once('close', resolve)));
      for (const socket of sockets) socket.destroy();
      await bounded(Promise.all([provider?.close(), ...closes]), 'owned socket cleanup');
      if (server?.listening) await bounded(new Promise(resolve => server.close(resolve)), 'fixture close');
    })();
    context.after(cleanup);
    const fixtureErrors = [];
    server = http.createServer((request, response) => {
      const begin = () => {
        response.writeHead(200, { 'content-type': 'text/event-stream' });
        response.write('data: {"synthetic":true}\n\n');
      };
      request.on('error', error => fixtureErrors.push(error.code));
      request.on('data', chunk => {
        chunks.push(chunk);
        if (early && !response.headersSent) begin();
      });
      request.on('end', () => {
        if (!response.headersSent) begin();
        response.end('data: [DONE]\n\n');
        ended.resolve();
      });
    });
    server.on('connection', socket => {
      sockets.add(socket);
      socket.once('close', () => sockets.delete(socket));
    });
    server.listen(0, '127.0.0.1');
    await bounded(once(server, 'listening'), 'fixture listener');
    const { port } = server.address();
    provider = createProvider({ upstream: `http://127.0.0.1:${port}`,
      headersTimeoutMs: 1000, inferenceTimeoutMs: 1000, idleTimeoutMs: 1000 });
    const source = {
      async next() {
        pulls += 1;
        if (pulls === 1) return { value: prefix, done: false };
        if (pulls === 2) {
          if (early) await release.promise;
          return { value: suffix, done: false };
        }
        return { done: true };
      },
      async return() { returns += 1; return { done: true }; },
      [Symbol.asyncIterator]() { return this; },
    };
    const result = await bounded(provider.forward({ path: '/v1/responses', body: source }), 'forward');
    release.resolve();
    const responseChunks = [];
    await bounded((async () => {
      for await (const chunk of result.body) responseChunks.push(chunk);
    })(), 'response completion');
    await bounded(ended.promise, 'upstream request completion');
    await cleanup();
    await refused(port);
    const received = Buffer.concat(chunks);
    context.diagnostic(JSON.stringify({ early, status: result.status, pulls, returns,
      expectedBytes: expected.length, receivedBytes: received.length,
      requestPreserved: received.equals(expected), responsePreserved:
        Buffer.concat(responseChunks).toString() === responseText,
      openSockets: sockets.size, fixturePortRefused: true, fixtureErrors }));
    assert.equal(result.status, 200);
    assert.equal(Buffer.concat(responseChunks).toString(), responseText);
    assert.equal(sockets.size, 0);
    assert.deepEqual(fixtureErrors, []);
    assert.deepEqual(received, expected, 'early success headers must not truncate the request');
    assert.equal(returns, 0, 'a successful complete upload must not be cancelled');
  });
}
