import assert from 'node:assert/strict';
import test from 'node:test';
import http from 'node:http';
import net from 'node:net';
import { getEventListeners, once } from 'node:events';
import { createProvider } from '../dist/provider.js';

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}
async function bounded(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolveUnused, reject) => {
      timer = setTimeout(() => reject(new Error('Provider fixture timeout')), 2000);
    })]);
  } finally { clearTimeout(timer); }
}
async function collect(body) {
  const chunks = [];
  for await (const chunk of body) chunks.push(chunk);
  return Buffer.concat(chunks).toString();
}
async function refused(port) {
  const socket = net.connect({ host: '127.0.0.1', port });
  try {
    const error = await bounded(new Promise(resolve => {
      socket.once('error', resolve);
      socket.once('connect', () => resolve(new Error('Listener still open')));
    }));
    assert.equal(error.code, 'ECONNREFUSED');
  } finally { socket.destroy(); }
}

for (const mode of ['rejected', 'cancelled', 'failed']) {
  test(`early response preserves ${mode} upload cleanup`, { timeout: 5000 }, async context => {
    const sockets = new Set();
    const release = deferred();
    const returned = deferred();
    const requestEnded = deferred();
    const received = [];
    const controller = new AbortController();
    let provider;
    let server;
    let response;
    let cleaning;
    let pulls = 0;
    let returns = 0;
    const cleanup = () => cleaning ??= (async () => {
      release.resolve();
      const closed = [...sockets].map(socket => new Promise(resolve => socket.once('close', resolve)));
      for (const socket of sockets) socket.destroy();
      await bounded(Promise.all([provider?.close(), ...closed]));
      if (server?.listening) await bounded(new Promise(resolve => server.close(resolve)));
    })();
    context.after(cleanup);
    server = http.createServer((request, outgoing) => {
      response = outgoing;
      request.on('error', () => {});
      request.on('end', requestEnded.resolve);
      request.on('data', chunk => {
        received.push(chunk);
        if (!outgoing.headersSent) {
          outgoing.writeHead(mode === 'rejected' ? 413 : 200, { 'content-type': 'text/event-stream' });
          outgoing.write('data: first\n\n');
        }
      });
    });
    server.on('connection', socket => {
      sockets.add(socket);
      socket.once('close', () => sockets.delete(socket));
    });
    server.listen(0, '127.0.0.1');
    await bounded(once(server, 'listening'));
    const { port } = server.address();
    provider = createProvider({ upstream: `http://127.0.0.1:${port}`,
      headersTimeoutMs: 1000, inferenceTimeoutMs: 1000, idleTimeoutMs: 1000 });
    const body = {
      async next() {
        pulls += 1;
        if (pulls === 1) return { done: false, value: Buffer.from('prefix') };
        await release.promise;
        if (mode === 'failed') throw new Error('synthetic upload failure');
        return { done: false, value: Buffer.from('unused suffix') };
      },
      async return() { returns += 1; returned.resolve(); return { done: true }; },
      [Symbol.asyncIterator]() { return this; },
    };
    const result = await bounded(provider.forward({ path: '/v1/responses', body,
      signal: controller.signal }));
    if (mode === 'rejected') {
      await bounded(returned.promise);
      await bounded(requestEnded.promise);
      response.end('data: rejected\n\n');
      assert.equal(result.status, 413);
      assert.equal(await bounded(collect(result.body)), 'data: first\n\ndata: rejected\n\n');
      assert.equal(Buffer.concat(received).toString(), 'prefix');
    } else {
      assert.equal(result.status, 200);
      const rejected = assert.rejects(collect(result.body), {
        code: mode === 'cancelled' ? 'request_cancelled' : 'request_body_error',
      });
      if (mode === 'cancelled') controller.abort();
      else release.resolve();
      await bounded(Promise.all([rejected, returned.promise]));
    }
    await cleanup();
    await refused(port);
    assert.equal(returns, 1);
    assert.equal(sockets.size, 0);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    context.diagnostic(JSON.stringify({ mode, returns, openSockets: sockets.size,
      abortListeners: 0, fixturePortRefused: true }));
  });
}
