import test from 'node:test';
import assert from 'node:assert/strict';
import { once, getEventListeners } from 'node:events';
import { createServer } from 'node:http';
import { createConnection } from 'node:net';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/remoteBackend.ts', import.meta.url))],
  alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) },
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { createRemoteBackend } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);
const ready = { apiVersion: 1, status: 'ready',
  capabilities: { responses: true, chatCompletions: true, streaming: true } };
const jsonReply = Buffer.from('{"ok":true}');
const streamReply = Buffer.from('data: {"ok":true}\n\n');

async function refused(port) {
  const socket = createConnection({ host: '127.0.0.1', port });
  try {
    const error = await new Promise((resolve, reject) => {
      socket.once('error', resolve);
      socket.once('connect', () => reject(new Error('fixture port remains open')));
      socket.setTimeout(500, () => reject(new Error('fixture port check timed out')));
    });
    assert.equal(error.code, 'ECONNREFUSED');
  } finally { socket.destroy(); }
}

for (const path of ['/v1/responses', '/v1/chat/completions',
  '/openai/v1/responses', '/anthropic/v1/messages']) {
  test(`Remote Node API filters Connection nominations on ${path}`, { timeout: 5000 }, async context => {
    const received = [];
    const sockets = new Set();
    const fixture = createServer((request, response) => {
      const chunks = [];
      request.on('data', chunk => chunks.push(chunk));
      request.on('end', () => {
        const body = Buffer.concat(chunks);
        received.push({ path: request.url, headers: request.headers, body });
        if (request.url === '/ready') {
          response.setHeader('content-type', 'application/json');
          response.end(JSON.stringify(ready));
          return;
        }
        const streaming = JSON.parse(body.toString()).stream;
        response.setHeader('content-type', streaming ? 'text/event-stream' : 'application/json');
        response.end(streaming ? streamReply : jsonReply);
      });
    });
    fixture.on('connection', socket => {
      sockets.add(socket);
      socket.once('close', () => sockets.delete(socket));
    });
    let backend;
    const cleanup = async () => {
      backend?.close();
      for (const socket of sockets) socket.destroy();
      if (fixture.listening) await new Promise(resolve => fixture.close(resolve));
    };
    context.after(cleanup);
    fixture.listen(0, '127.0.0.1');
    await once(fixture, 'listening');
    const port = fixture.address().port;
    backend = createRemoteBackend({
      endpoint: { origin: `http://127.0.0.1:${port}`, loopback: true },
      serverToken: 'synthetic-server-token', headersTimeoutMs: 1000,
      idleTimeoutMs: 1000, readinessWholeMs: 1000, readinessBodyMaxBytes: 65536,
      inferenceTimeoutMs: 2000,
    });
    const observations = [];
    for (const stream of [false, true]) {
      for (const nominated of [false, true]) {
        const body = Buffer.from(JSON.stringify({ model: 'synthetic', stream }));
        const signal = AbortSignal.timeout(2000);
        const response = await backend.forward({ path, signal,
          headers: { accept: 'synthetic/accept', 'content-type': 'application/json',
            'openai-beta': 'synthetic-beta', 'anthropic-version': '2023-06-01',
            'anthropic-beta': 'synthetic-anthropic', cookie: 'synthetic-cookie',
            authorization: 'Bearer synthetic-client-token',
            ...(nominated ? { connection: ' AcCePt, OPENAI-BETA, anthropic-beta, authorization ' } : {}) },
          body: (async function* () { yield body; })(),
        });
        const chunks = [];
        for await (const chunk of response.body) chunks.push(chunk);
        assert.equal(response.status, 200);
        assert.deepEqual(Buffer.concat(chunks), stream ? streamReply : jsonReply);
        assert.equal(getEventListeners(signal, 'abort').length, 0);
        observations.push({ nominated, body, received: received.at(-1) });
      }
    }
    await cleanup();
    await refused(port);
    context.diagnostic('all response bytes consumed; owned fixture closed and port refused');
    const completed = path.startsWith('/openai/') || path.startsWith('/anthropic/');
    assert.deepEqual(received.map(item => item.path),
      Array.from({ length: 4 }, () => completed ? [path] : ['/ready', path]).flat());
    for (const observation of observations) {
      const headers = observation.received.headers;
      assert.deepEqual(observation.received.body, observation.body);
      assert.equal(headers.authorization, 'Bearer synthetic-server-token');
      assert.equal(headers.cookie, undefined);
      assert.equal(headers['content-type'], 'application/json');
      assert.equal(headers['anthropic-version'], completed ? '2023-06-01' : undefined);
      assert.equal(headers.accept, observation.nominated ? undefined : 'synthetic/accept');
      assert.equal(headers['openai-beta'], observation.nominated ? undefined : 'synthetic-beta');
      assert.equal(headers['anthropic-beta'], completed && !observation.nominated ? 'synthetic-anthropic' : undefined);
    }
  });
}
