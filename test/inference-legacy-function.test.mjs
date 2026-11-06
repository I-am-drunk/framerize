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
}, bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
const bytes = compiled.outputFiles[0].contents;
const { checkInference, inspectCompletion } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
console.log(JSON.stringify({ runtime: process.version, platform: process.platform,
  inputs: Object.keys(compiled.metafile.inputs).sort(),
  sourceBundleSha256: createHash('sha256').update(bytes).digest('hex') }));

const chunk = (delta, finish_reason = null) =>
  `data: ${JSON.stringify({ choices: [{ index: 0, delta, finish_reason }] })}\n\n`;
const done = 'data: [DONE]\n\n';
const complete = chunk({ content: 'OK' }) + chunk({}, 'stop') + done;
const call = { name: 'synthetic_tool', arguments: '{"synthetic":true}' };
const cases = [
  ['legacy call preceding text', chunk({ function_call: call }) + complete, false],
  ['legacy call alongside text', chunk({ content: 'OK', function_call: call }) + chunk({}, 'stop') + done, false],
  ['ordinary text', complete, true],
  ['modern tool call', chunk({ tool_calls: [{ index: 0, function: call }] }) + complete, false],
  ['refusal', chunk({ refusal: 'synthetic refusal' }) + complete, false],
  ['legacy call terminal reason', chunk({ function_call: call }) + chunk({}, 'function_call') + done, false],
];

for (const [name, text, expectedOk] of cases) {
  test(`text-only checker: ${name}`, { timeout: 5000 }, async context => {
    const sockets = new Set();
    const closed = [];
    const requests = [];
    const controller = new AbortController();
    const server = createServer((request, response) => {
      const body = [];
      request.on('data', value => body.push(value));
      request.on('end', () => {
        requests.push({ method: request.method, path: request.url,
          body: JSON.parse(Buffer.concat(body).toString()) });
        response.writeHead(200, { 'content-type': 'text/event-stream' });
        response.end(text);
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
    let directOk = true;
    try { inspectCompletion(Buffer.from(text)); }
    catch { directOk = false; }
    let result;
    try {
      result = await checkInference({ endpoint: `http://127.0.0.1:${port}`,
        token: 'synthetic-token', model: 'synthetic-model', timeoutMs: 2500,
        signal: controller.signal });
      await bounded(Promise.all(closed));
      assert.equal(sockets.size, 0);
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    } finally { await cleanup(); }
    await refuses(port);
    assert.equal(requests.length, 1);
    assert.equal(requests[0].method, 'POST');
    assert.equal(requests[0].path, '/v1/chat/completions');
    assert.equal(requests[0].body.model, 'synthetic-model');
    assert.equal(requests[0].body.stream, true);
    assert.equal(result.requestAttempts, 1);
    assert.equal(result.status, 200);
    context.diagnostic(JSON.stringify({ expectedOk, directOk, result, portRefused: true }));
    assert.equal(result.ok, expectedOk, 'tool-bearing output must not qualify as text-only inference');
    assert.equal(directOk, expectedOk);
    if (!expectedOk) assert.equal(result.code, 'invalid_stream');
  });
}

async function bounded(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error('synthetic fixture timed out')), 1500);
    })]);
  } finally { clearTimeout(timer); }
}

async function refuses(port) {
  const socket = createConnection({ host: '127.0.0.1', port });
  const closed = new Promise(resolve => socket.once('close', resolve));
  try {
    const error = await bounded(new Promise((resolve, reject) => {
      socket.once('error', resolve);
      socket.once('connect', () => reject(new Error('fixture port remains open')));
    }));
    assert.equal(error.code, 'ECONNREFUSED');
  } finally {
    socket.destroy();
    await bounded(closed);
  }
}
