import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createConnection } from 'node:net';
import { once, getEventListeners } from 'node:events';
import { createHash } from 'node:crypto';
import { gzipSync } from 'node:zlib';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(
  new URL('../scripts/validation/inference.ts', import.meta.url))],
bundle: true, platform: 'node', format: 'esm', write: false });
const bytes = compiled.outputFiles[0].contents;
const bundleHash = createHash('sha256').update(bytes).digest('hex');
const { checkInference } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

async function bounded(work, label) {
  let timer;
  try {
    return await Promise.race([work, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(label)), 1500);
    })]);
  } finally { clearTimeout(timer); }
}

async function refused(port) {
  const socket = createConnection({ host: '127.0.0.1', port });
  try {
    const [error] = await bounded(once(socket, 'error'), 'fixture port still open');
    assert.equal(error.code, 'ECONNREFUSED');
  } finally { socket.destroy(); }
}

function reply(protocol) {
  if (protocol === 'chat') {
    const frame = (delta, finish_reason) => `data: ${JSON.stringify({
      choices: [{ index: 0, delta, finish_reason }],
    })}\n\n`;
    return Buffer.from(frame({ content: 'OK' }, null) + frame({}, 'stop') + 'data: [DONE]\n\n');
  }
  return Buffer.from(JSON.stringify(protocol === 'responses'
    ? { id: 'synthetic-response', object: 'response', model: 'fixture', created_at: 1,
      status: 'completed', output: [{ id: 'synthetic-message', type: 'message', role: 'assistant',
        content: [{ type: 'output_text', text: 'OK' }] }] }
    : { id: 'synthetic-message', type: 'message', model: 'fixture', role: 'assistant',
      content: [{ type: 'text', text: 'OK' }], stop_reason: 'end_turn' }));
}

for (const protocol of ['chat', 'responses', 'messages']) {
  for (const coding of [undefined, 'identity', 'IDENTITY', 'iDeNtItY', 'gzip']) {
    test(`${protocol} inference check handles ${coding ?? 'absent'} coding`, { timeout: 5000 }, async context => {
      const payload = reply(protocol);
      const requests = [], failures = [], sockets = new Set(), closedSockets = [];
      const controller = new AbortController();
      const server = createServer((request, response) => {
        requests.push([request.method, request.url]);
        let body = '';
        request.setEncoding('utf8');
        request.on('data', chunk => { body += chunk; });
        request.on('end', () => {
          try {
            assert.equal(request.headers.authorization, 'Bearer synthetic-check-token');
            const decoded = JSON.parse(body);
            assert.equal(decoded.model, 'fixture');
            assert.equal(decoded.stream, protocol === 'chat');
          } catch (error) { failures.push(error); }
          response.writeHead(200, { 'content-type': protocol === 'chat'
            ? 'text/event-stream' : 'application/json', connection: 'close',
          ...(coding ? { 'content-encoding': coding } : {}) });
          response.end(coding === 'gzip' ? gzipSync(payload) : payload);
        });
      });
      let shutdown;
      const close = () => shutdown ??= (async () => {
        for (const socket of sockets) socket.destroy();
        if (server.listening) await bounded(new Promise((resolve, reject) =>
          server.close(error => error ? reject(error) : resolve())), 'fixture close timed out');
      })();
      context.after(close);
      server.on('connection', socket => {
        sockets.add(socket);
        closedSockets.push(new Promise(resolve => socket.once('close', () => {
          sockets.delete(socket);
          resolve();
        })));
      });
      server.listen(0, '127.0.0.1');
      await bounded(once(server, 'listening'), 'fixture listen timed out');
      const port = server.address().port;
      const result = await checkInference({ endpoint: `http://127.0.0.1:${port}`,
        token: 'synthetic-check-token', model: 'fixture', protocol,
        timeoutMs: 3000, signal: controller.signal });
      await bounded(Promise.all(closedSockets), 'checker retained a socket');
      assert.equal(sockets.size, 0);
      await close();
      await refused(port);
      context.diagnostic(JSON.stringify({ bundleHash, protocol, coding: coding ?? null,
        result, requests: requests.length, remainingSockets: sockets.size, cleanup: 'refused' }));
      assert.deepEqual(failures, []);
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
      assert.equal(controller.signal.aborted, false);
      const route = protocol === 'chat' ? '/v1/chat/completions'
        : protocol === 'responses' ? '/openai/v1/responses' : '/anthropic/v1/messages';
      assert.deepEqual(requests, [['POST', route]]);
      assert.equal(result.requestAttempts, 1);
      assert.equal(result.status, 200);
      if (coding === 'gzip') {
        assert.equal(result.ok, false);
        assert.equal(result.code, 'unsupported_encoding');
      } else {
        assert.equal(result.ok, true);
        assert.equal(result.protocol, protocol);
        assert.equal(result.contentBytes, 2);
        assert.equal(result.responseBytes, payload.length);
      }
    });
  }
}
