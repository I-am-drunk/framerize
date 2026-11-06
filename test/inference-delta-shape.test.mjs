import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createConnection } from 'node:net';
import { once, getEventListeners } from 'node:events';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { checkInference } from './inference.ts'; export { inspectCompletion } from './completion.ts';",
  resolveDir: fileURLToPath(new URL('../scripts/validation/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false });
const bytes = compiled.outputFiles[0].contents;
const bundleHash = createHash('sha256').update(bytes).digest('hex');
const { checkInference, inspectCompletion } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

async function bounded(work) {
  let timer;
  try {
    return await Promise.race([work, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error('fixture operation timed out')), 1500);
    })]);
  } finally { clearTimeout(timer); }
}

const frame = (delta, finish_reason = null) => `data: ${JSON.stringify({
  id: 'chatcmpl-synthetic', object: 'chat.completion.chunk', created: 1, model: 'fixture',
  choices: [{ index: 0, delta, finish_reason }],
})}\n\n`;

const cases = [['empty initial', [], false], ['populated initial', [{}], false],
  ['empty terminal', [], true], ['populated terminal', [{ content: 'invalid' }], true],
  ['ordinary object', {}, true]];

for (const [name, delta, terminal] of cases) {
  test(`chat checker validates ${name} delta`, { timeout: 5000 }, async context => {
    const payload = Buffer.from((terminal ? '' : frame(delta))
      + frame({ role: 'assistant', content: 'OK' })
      + frame(terminal ? delta : {}, 'stop') + 'data: [DONE]\n\n');
    let directResult, directError;
    try { directResult = inspectCompletion(payload); }
    catch (error) { directError = error.message; }
    const requests = [], failures = [], sockets = new Set(), closures = [];
    const controller = new AbortController();
    const server = createServer((request, response) => {
      requests.push([request.method, request.url]);
      let body = '';
      request.setEncoding('utf8');
      request.on('data', chunk => { body += chunk; });
      request.on('end', () => {
        try {
          assert.equal(request.headers.authorization, 'Bearer synthetic-check-token');
          assert.equal(JSON.parse(body).model, 'fixture');
          assert.equal(JSON.parse(body).stream, true);
        } catch (error) { failures.push(error); }
        response.writeHead(200, { 'content-type': 'text/event-stream', connection: 'close' });
        response.end(payload);
      });
    });
    let closing;
    const close = () => closing ??= (async () => {
      for (const socket of sockets) socket.destroy();
      if (server.listening) await bounded(new Promise((resolve, reject) =>
        server.close(error => error ? reject(error) : resolve())));
    })();
    context.after(close);
    server.on('connection', socket => {
      sockets.add(socket);
      closures.push(new Promise(resolve => socket.once('close', () => {
        sockets.delete(socket);
        resolve();
      })));
    });
    server.listen(0, '127.0.0.1');
    await bounded(once(server, 'listening'));
    const port = server.address().port;
    const result = await checkInference({ endpoint: `http://127.0.0.1:${port}`,
      token: 'synthetic-check-token', model: 'fixture', timeoutMs: 3000,
      signal: controller.signal });
    await bounded(Promise.all(closures));
    assert.equal(sockets.size, 0);
    await close();
    const socket = createConnection({ host: '127.0.0.1', port });
    try {
      const [error] = await bounded(once(socket, 'error'));
      assert.equal(error.code, 'ECONNREFUSED');
    } finally { socket.destroy(); }
    context.diagnostic(JSON.stringify({ bundleHash, directResult, directError, result,
      requests: requests.length, remainingSockets: sockets.size, cleanup: 'refused' }));
    assert.deepEqual(failures, []);
    assert.deepEqual(requests, [['POST', '/v1/chat/completions']]);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    assert.equal(controller.signal.aborted, false);
    assert.equal(result.status, 200);
    assert.equal(result.requestAttempts, 1);
    if (Array.isArray(delta)) {
      assert.equal(result.ok, false);
      assert.equal(result.code, 'invalid_stream');
      assert.equal(directError, 'Invalid choice');
      assert.equal(directResult, undefined);
    } else {
      assert.equal(result.ok, true);
      assert.equal(result.contentBytes, 2);
      assert.equal(directError, undefined);
      assert.deepEqual(directResult, { events: 2, contentBytes: 2 });
    }
  });
}
