import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createConnection } from 'node:net';
import { once, getEventListeners } from 'node:events';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: 'export * from "./validation/inference.ts"; export * from "./validation/completion.ts";',
  resolveDir: fileURLToPath(new URL('../scripts/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false });
const bytes = compiled.outputFiles[0].contents;
const { checkInference, inspectCompletion } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
console.log(JSON.stringify({ runtime: process.version, platform: process.platform,
  sourceBundleSha256: createHash('sha256').update(bytes).digest('hex') }));

const chunk = (delta, finish_reason = null) =>
  `data: ${JSON.stringify({ choices: [{ index: 0, delta, finish_reason }] })}\n\n`;
const complete = chunk({ content: 'OK' }) + chunk({}, 'stop') + 'data: [DONE]\n\n';
const cases = [
  ['no-data error field before completion', 'event: error\n\n' + complete, true],
  ['no-data error field after completion', complete + 'event:error\n\n', true],
  ['event field overwritten with message', 'event: error\nevent: message\n' + complete, true],
  ['event field reset without a colon', 'event: error\nevent\n' + complete, true],
  ['ordinary message event', 'event: message\n' + complete, true],
  ['last event field selects error', 'event: message\nevent: error\n' + complete, false],
  ['dispatched error without space', 'event:error\n' + complete, false],
  ['JSON error inside a message event', 'event: message\ndata: {"error":"synthetic"}\n\n' + complete, false],
];

async function bounded(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error('Fixture did not close in time')), 1500);
    })]);
  } finally { clearTimeout(timer); }
}

async function refused(port) {
  const socket = createConnection({ host: '127.0.0.1', port });
  try {
    const error = await bounded(new Promise((resolve, reject) => {
      socket.once('error', resolve);
      socket.once('connect', () => reject(new Error('Fixture still accepts connections')));
    }));
    assert.equal(error.code, 'ECONNREFUSED');
  } finally { socket.destroy(); }
}

for (const [name, text, accepted] of cases) {
  test(`SSE dispatch respects ${name}`, { timeout: 5000 }, async context => {
    const reply = Buffer.from(text);
    const sockets = new Set();
    const closed = [];
    const requests = [];
    let result, inspection, inspectionError;
    const controller = new AbortController();
    const server = createServer((request, response) => {
      const body = [];
      request.on('data', value => body.push(value));
      request.on('end', () => {
        requests.push({ method: request.method, path: request.url,
          headers: request.headers, body: Buffer.concat(body) });
        response.writeHead(200, { 'content-type': 'text/event-stream' });
        response.end(reply);
      });
    });
    server.on('connection', socket => {
      sockets.add(socket);
      closed.push(new Promise(resolve => socket.once('close', () => {
        sockets.delete(socket);
        resolve();
      })));
    });
    const cleanup = async () => {
      controller.abort();
      for (const socket of sockets) socket.destroy();
      if (server.listening) await bounded(new Promise(resolve => server.close(resolve)));
      await bounded(Promise.all(closed));
    };
    context.after(cleanup);
    server.listen(0, '127.0.0.1');
    await bounded(once(server, 'listening'));
    const port = server.address().port;
    try { inspection = inspectCompletion(reply); }
    catch (error) { inspectionError = error.message; }
    try {
      result = await checkInference({ endpoint: `http://127.0.0.1:${port}`,
        token: 'synthetic-token', model: 'synthetic-model', timeoutMs: 3000,
        signal: controller.signal });
      await bounded(Promise.all(closed));
      assert.equal(sockets.size, 0);
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    } finally {
      await cleanup();
      await refused(port);
    }
    assert.equal(requests.length, 1);
    assert.equal(requests[0].method, 'POST');
    assert.equal(requests[0].path, '/v1/chat/completions');
    assert.equal(requests[0].headers.authorization, 'Bearer synthetic-token');
    assert.deepEqual(JSON.parse(requests[0].body), { model: 'synthetic-model', stream: true,
      messages: [{ role: 'user', content: 'Reply with exactly OK.' }] });
    context.diagnostic(JSON.stringify({ inspectionError, httpOk: result.ok,
      httpCode: result.code, requestAttempts: result.requestAttempts,
      socketsClosed: true, portRefused: true }));
    assert.equal(result.requestAttempts, 1);
    assert.equal(result.status, 200);
    if (accepted) {
      assert.equal(inspectionError, undefined);
      assert.deepEqual(inspection, { events: 2, contentBytes: 2 });
      assert.equal(result.ok, true);
      assert.equal(result.events, 2);
      assert.equal(result.contentBytes, 2);
    } else {
      assert.equal(typeof inspectionError, 'string');
      assert.equal(result.ok, false);
      assert.equal(result.code, 'invalid_stream');
    }
  });
}
