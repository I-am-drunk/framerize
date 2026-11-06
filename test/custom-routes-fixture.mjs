import assert from 'node:assert/strict';
import { Agent, createServer as httpServer, request } from 'node:http';
import { once, getEventListeners } from 'node:events';
import { createServer } from '../framerize-server/dist/server.js';
import { createFramerBackend } from '../framerize-server/dist/framer-backend.js';
import { createRemoteBackend } from '../framerize-remote/dist/index.js';

export function post(origin, path, body, agent, headers = {}, signal, onChunk) {
  return new Promise((resolve, reject) => {
    const outgoing = request(new URL(path, origin), { agent, method: 'POST', signal,
      headers: { authorization: 'Bearer synthetic-client', 'content-type': 'application/json', ...headers },
    }, response => {
      const chunks = [];
      response.on('data', chunk => { chunks.push(chunk); onChunk?.(chunk); });
      response.once('error', reject);
      response.once('end', () => resolve({ status: response.statusCode,
        headers: response.headers, body: Buffer.concat(chunks) }));
    });
    outgoing.setTimeout(3000, () => outgoing.destroy(new Error('fixture deadline')));
    outgoing.once('error', reject);
    outgoing.end(typeof body === 'string' ? body : JSON.stringify(body));
  });
}

export const protocols = [
  { name: 'responses', path: '/openai/v1/responses', upstream: '/ai/v3/responses/',
    body: { model: 'synthetic-model', input: 'Synthetic request.', stream: false } },
  { name: 'messages', path: '/anthropic/v1/messages', upstream: '/ai/v3/messages/',
    body: { model: 'synthetic-model', messages: [{ role: 'user', content: 'Synthetic request.' }],
      max_tokens: 32, stream: false } },
];

export function reply(protocol, overrides = {}) {
  const common = { id: 'synthetic-reply', model: 'synthetic-model',
    usage: { input_tokens: 1, output_tokens: 2 } };
  return protocol.name === 'responses'
    ? { ...common, object: 'response', created_at: 1, status: 'completed',
      output: [{ type: 'message', id: 'synthetic-message', role: 'assistant',
        content: [{ type: 'output_text', text: 'Synthetic reply.', annotations: [] }] }], ...overrides }
    : { ...common, type: 'message', role: 'assistant', stop_reason: 'end_turn',
      stop_sequence: null, content: [{ type: 'text', text: 'Synthetic reply.' }], ...overrides };
}

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

export async function fixture(context, protocol, viaRemote, options = {}) {
  const observed = [];
  const signals = [];
  const sockets = new Set();
  const closures = [];
  const entered = deferred();
  const disconnected = deferred();
  const counts = { auth: 0, mints: 0, authClose: 0, castleClose: 0 };
  const payload = Buffer.from(options.payload ?? JSON.stringify(options.value ?? reply(protocol)));
  const upstream = httpServer(async (incoming, response) => {
    const chunks = [];
    for await (const chunk of incoming) chunks.push(chunk);
    observed.push({ path: incoming.url, headers: incoming.headers,
      body: JSON.parse(Buffer.concat(chunks)) });
    response.once('close', disconnected.resolve);
    entered.resolve();
    if (options.respond) { options.respond(response, observed.length); return; }
    response.writeHead(options.status ?? 200, {
      'content-type': 'application/json', ...options.headers,
      ...(options.mode === 'length' ? { 'content-length': payload.length + 100 } : {}),
    });
    if (options.mode === 'stall') { response.write(payload.subarray(0, 1)); return; }
    if (options.mode === 'chunked') {
      const socket = response.socket;
      response.write(payload, () => socket.end());
    } else response.end(payload);
  });
  upstream.on('connection', socket => {
    sockets.add(socket);
    closures.push(new Promise(resolve => socket.once('close', resolve)));
    socket.once('close', () => sockets.delete(socket));
  });
  const agent = new Agent({ keepAlive: false, proxyEnv: {} });
  let server;
  let remote;
  context.after(async () => {
    agent.destroy();
    let forced = false;
    const emergency = setTimeout(() => {
      forced = true;
      for (const socket of sockets) socket.destroy();
    }, 2000);
    try {
      await remote?.close();
      await server?.close();
      await new Promise(resolve => upstream.close(resolve));
      await Promise.all(closures);
      assert.equal(forced, false, 'cleanup must not require forced socket closure');
      assert.equal(sockets.size, 0);
      assert.equal(counts.authClose, 1);
      assert.equal(counts.castleClose, 1);
      for (const signal of signals) assert.equal(getEventListeners(signal, 'abort').length, 0);
    } finally { clearTimeout(emergency); }
  });
  upstream.listen(0, '127.0.0.1');
  await once(upstream, 'listening');
  const backend = createFramerBackend({ projectId: 'synthetic-project',
    origin: `http://127.0.0.1:${upstream.address().port}`,
    headersTimeoutMs: 1000, inferenceTimeoutMs: 2000, idleTimeoutMs: 1000,
    ...options.backend,
    auth: { async accessToken() { counts.auth += 1; return 'synthetic-upstream'; },
      async close() { counts.authClose += 1; } },
    castle: { async mint({ signal }) { signals.push(signal); counts.mints += 1;
      return { token: 'synthetic-castle' }; },
    async close() { counts.castleClose += 1; } },
  });
  const limits = { headersTimeoutMs: 1000, inferenceTimeoutMs: 2500, idleTimeoutMs: 1000 };
  server = await createServer({ host: '127.0.0.1', port: 0, ...limits, ...options.server,
    serverToken: viaRemote ? 'synthetic-server' : 'synthetic-client' }, { provider: backend });
  let origin = `http://127.0.0.1:${(await server.listen()).port}`;
  const serverCalls = [];
  server.server.on('request', incoming => serverCalls.push({ path: incoming.url,
    authorization: incoming.headers.authorization }));
  if (viaRemote) {
    const forwarding = createRemoteBackend({ endpoint: { origin, loopback: true },
      serverToken: 'synthetic-server', ...limits, readinessWholeMs: 1000,
      readinessBodyMaxBytes: 65536, ...options.remote });
    remote = await createServer({ host: '127.0.0.1', port: 0, ...limits,
      serverToken: 'synthetic-client' }, { provider: forwarding });
    origin = `http://127.0.0.1:${(await remote.listen()).port}`;
  }
  return { origin, payload, observed, counts, serverCalls, listener: remote?.server ?? server.server,
    entered: entered.promise,
    disconnected: disconnected.promise, signals,
    post: (body = protocol.body, headers, signal) =>
      post(origin, protocol.path, body, agent, headers, signal, options.onChunk),
  };
}
