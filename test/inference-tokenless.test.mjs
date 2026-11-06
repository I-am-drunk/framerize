import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createConnection } from 'node:net';
import { once, getEventListeners } from 'node:events';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const entry = new URL('../scripts/check-inference.ts', import.meta.url);
const compiled = await build({ entryPoints: [fileURLToPath(entry)],
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
  define: { 'import.meta.url': JSON.stringify(entry.href) } });
const bytes = compiled.outputFiles[0].contents;
const { runInferenceCheck } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
console.log(JSON.stringify({ runtime: process.version, platform: process.platform,
  entryUrl: entry.href, inputs: Object.keys(compiled.metafile.inputs).sort(),
  bundleSha256: createHash('sha256').update(bytes).digest('hex') }));

const model = 'synthetic-model';
const responses = { id: 'synthetic-response', object: 'response', status: 'completed',
  created_at: 1, model, output: [{ type: 'message', id: 'synthetic-message',
    role: 'assistant', status: 'completed', content: [{ type: 'output_text', text: 'OK' }] }] };
const messages = { id: 'synthetic-message', type: 'message', role: 'assistant', model,
  content: [{ type: 'text', text: 'OK' }], stop_reason: 'end_turn', stop_sequence: null };
const chat = `data: ${JSON.stringify({ choices: [{ index: 0,
  delta: { content: 'OK' }, finish_reason: 'stop' }] })}\n\ndata: [DONE]\n\n`;
const replies = { chat, responses: JSON.stringify(responses), messages: JSON.stringify(messages) };
const paths = { chat: '/v1/chat/completions', responses: '/openai/v1/responses',
  messages: '/anthropic/v1/messages' };
const cases = Object.keys(replies).flatMap(protocol =>
  ['tokenless', 'environment token'].map(mode => ({ protocol, mode })));
cases.push({ protocol: 'chat', mode: 'missing explicit token' },
  { protocol: 'chat', mode: 'missing consent' });

for (const { protocol, mode } of cases) {
  test(`inference CLI ${protocol}: ${mode}`, { timeout: 5000 }, async context => {
    const requests = [];
    const sockets = new Set();
    const closed = [];
    const controller = new AbortController();
    const server = createServer((request, response) => {
      const body = [];
      request.on('data', value => body.push(value));
      request.on('end', () => {
        requests.push({ path: request.url, method: request.method,
          authorization: request.headers.authorization,
          body: JSON.parse(Buffer.concat(body).toString()) });
        response.writeHead(200, { 'content-type': protocol === 'chat'
          ? 'text/event-stream' : 'application/json' });
        response.end(replies[protocol]);
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
    const argv = ['--endpoint', `http://127.0.0.1:${port}`, '--model', model,
      '--protocol', protocol, '--timeout-ms', '2000'];
    if (mode !== 'missing consent') argv.push('--send-one-request');
    if (mode === 'environment token' || mode === 'missing consent') argv.push('--token-env', 'SYNTHETIC_TOKEN');
    if (mode === 'missing explicit token') argv.push('--token-env', 'MISSING_TOKEN');
    const env = Object.freeze(mode === 'environment token' || mode === 'missing consent'
      ? { SYNTHETIC_TOKEN: 'synthetic-token' } : {});
    let result;
    try {
      result = await runInferenceCheck(argv, env, controller.signal);
      await bounded(Promise.all(closed));
      assert.equal(sockets.size, 0);
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    } finally { await cleanup(); }
    await refuses(port);
    const expectedOk = mode === 'tokenless' || mode === 'environment token';
    context.diagnostic(JSON.stringify({ protocol, mode, result,
      requests: requests.length, portRefused: true }));
    assert.equal(result.ok, expectedOk, 'the checker must support optional endpoint authentication');
    assert.equal(result.requestAttempts, expectedOk ? 1 : 0);
    assert.equal(requests.length, expectedOk ? 1 : 0);
    if (expectedOk) {
      assert.equal(result.status, 200);
      assert.equal(result.protocol, protocol);
      assert.equal(requests[0].method, 'POST');
      assert.equal(requests[0].path, paths[protocol]);
      assert.equal(requests[0].body.model, model);
      assert.equal(requests[0].body.stream, protocol === 'chat');
      assert.equal(requests[0].authorization,
        mode === 'environment token' ? 'Bearer synthetic-token' : undefined);
    } else {
      assert.equal(result.status, null);
      assert.equal(result.code, mode === 'missing explicit token' ? 'credential' : 'configuration');
    }
    assert.deepEqual(env, mode === 'environment token' || mode === 'missing consent'
      ? { SYNTHETIC_TOKEN: 'synthetic-token' } : {});
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
