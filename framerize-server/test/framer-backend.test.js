import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer as httpServer } from 'node:http';
import { once, getEventListeners } from 'node:events';
import { createFramerAuth } from '../dist/framer-auth.js';
import { createFramerBackend } from '../dist/framer-backend.js';
import { createServer } from '../dist/server.js';
import { collect, deferred } from './helpers/fixtures.js';

const token = 'synthetic-framer-token';
const cookie = 'session=synthetic-private-cookie';
const body = { model: 'example/model', messages: [{ role: 'user', content: 'hello' }], stream: true };
const signal = () => new AbortController().signal;

for (const path of ['/openai/v1/responses', '/anthropic/v1/messages']) {
  for (const action of ['return', 'abort', 'close']) {
    test(`${action} cleans an unread converted stream: ${path}`, { timeout: 4000 }, async context => {
      const entered = deferred();
      const closed = deferred();
      const setup = await fixture(context, (request, response) => {
        response.once('close', closed.resolve);
        entered.resolve();
      });
      const controller = new AbortController();
      const value = path === '/openai/v1/responses'
        ? { model: 'example', input: 'hello', stream: true }
        : { ...body, max_tokens: 32 };
      const result = await setup.backend.forward({ ...call(value, controller.signal), path });
      await entered.promise;
      if (action === 'return') await result.body.return();
      else if (action === 'close') await setup.backend.close();
      else controller.abort();
      await closed.promise;
      await setup.backend.close();
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
      assert.equal(setup.closed(), 1);
      assert.equal(setup.mints(), 1);
    });
  }
}

test('Framer inference gets a longer wait after bounded preparation', { timeout: 3000 }, async context => {
  const setup = await fixture(context, (request, response) => {
    const timer = setTimeout(() => response.end('{"done":true}'), 240);
    response.once('close', () => clearTimeout(timer));
  }, { headersTimeoutMs: 80, idleTimeoutMs: 80, inferenceTimeoutMs: 1000 });
  const controller = new AbortController();
  const result = await setup.backend.forward(call({ ...body, stream: false }, controller.signal));
  assert.equal(result.status, 200);
  assert.equal((await collect(result.body)).toString(), '{"done":true}');
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});
const call = (value = body, parent = signal()) => ({ path: '/v1/chat/completions', signal: parent,
  headers: { authorization: 'Bearer synthetic-local-key', cookie: 'must-not-forward' },
  body: (async function* () { yield Buffer.from(JSON.stringify(value)); })(),
});

async function fixture(context, handler = (request, response) => response.end('{}'), overrides = {}) {
  const requests = [];
  const gateway = httpServer(async (request, response) => {
    const bytes = await collect(request);
    requests.push({ path: request.url, headers: request.headers, bytes });
    if (request.url === '/auth/ai/access-token') {
      response.setHeader('content-type', 'application/json');
      response.end(JSON.stringify({ accessToken: token, expiresAt: new Date(Date.now() + 600_000).toISOString() }));
    } else handler(request, response);
  });
  gateway.listen(0, '127.0.0.1');
  await once(gateway, 'listening');
  const origin = `http://127.0.0.1:${gateway.address().port}`;
  let mints = 0;
  let closed = 0;
  const auth = createFramerAuth({ sessionCookie: cookie, origin });
  const config = { origin, projectId: 'synthetic-project', auth,
    castle: { async mint({ signal }) { signal.throwIfAborted(); mints += 1; return { token: 'synthetic-castle' }; },
      async close() { closed += 1; } }, ...overrides };
  const backend = createFramerBackend(config);
  context.after(async () => {
    await backend.close();
    await auth.close();
    gateway.closeAllConnections();
    await new Promise(resolve => gateway.close(resolve));
  });
  return { backend, requests, config, gateway, mints: () => mints, closed: () => closed };
}

test('Framer readiness uses session auth without minting or /v1/models', async context => {
  const setup = await fixture(context);
  assert.equal(await setup.backend.ready({ signal: signal() }), true);
  assert.equal(await setup.backend.ready({ signal: signal() }), true);
  assert.deepEqual(setup.requests.map(request => request.path), ['/auth/ai/access-token']);
  assert.equal(setup.requests[0].headers.cookie, cookie);
  assert.equal(setup.mints(), 0);
  const closing = setup.backend.close();
  assert.equal(setup.backend.close(), closing);
  await closing;
  assert.equal(setup.closed(), 1);
  await assert.rejects(setup.backend.ready({ signal: signal() }), { code: 'provider_closed' });
});

for (const path of ['/v1/responses', '/v1/chat/completions']) {
  test(`${path} forwards its native envelope and exact stream with isolated credentials`, async context => {
    const output = Buffer.from('data: {"choices":[{"delta":{"content":"hello"}}]}\n\ndata: [DONE]\n\n');
    const setup = await fixture(context, (request, response) => {
      response.writeHead(200, { 'content-type': 'text/event-stream' });
      response.write(output.subarray(0, 9)); response.end(output.subarray(9));
    });
    const controller = new AbortController();
    const input = path === '/v1/responses' ? { model: 'example', input: 'hello', stream: true } : body;
    const result = await setup.backend.forward({ ...call(input, controller.signal), path });
    assert.equal(result.status, 200);
    assert.deepEqual(await collect(result.body), output);
    const sent = setup.requests[1];
    assert.equal(sent.path, path === '/v1/responses' ? '/ai/v3/openai/responses/' : '/ai/v3/chat/');
    assert.equal(sent.headers.authorization, `Bearer ${token}`);
    assert.equal(sent.headers.cookie, undefined);
    const envelope = JSON.parse(sent.bytes);
    assert.deepEqual(envelope.request, input);
    assert.equal(envelope.castleToken, 'synthetic-castle');
    assert.equal(envelope.featureCategory, 'agents');
    assert.equal(setup.mints(), 1);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  });
}

for (const status of [401, 402, 403, 429, 500]) {
  test(`HTTP ${status} retains its body without retrying or retiring the session`, async context => {
    const setup = await fixture(context, (request, response) => {
      response.writeHead(status); response.end('synthetic-private-upstream-error');
    });
    const result = await setup.backend.forward(call());
    assert.equal(result.status, status);
    assert.equal((await collect(result.body)).toString(), 'synthetic-private-upstream-error');
    assert.equal(setup.requests.length, 2);
    assert.equal(await setup.backend.ready({ signal: signal() }), true);
    assert.equal(setup.requests.length, 2);
  });
}

test('bad input, oversized bodies and pre-abort use no auth, mint or network', async context => {
  const setup = await fixture(context, undefined, { maxBodyBytes: 150 });
  await assert.rejects(setup.backend.forward(call({})), { code: 'invalid_request_body' });
  await assert.rejects(setup.backend.forward(call({ ...body, extra: 'x'.repeat(200) })), { code: 'body_too_large' });
  await assert.rejects(setup.backend.forward(call(body, AbortSignal.abort())), { code: 'request_cancelled' });
  assert.equal(setup.mints(), 0);
  assert.equal(setup.requests.length, 0);
});

test('non-streaming tool requests work through the actual authenticated listener', async context => {
  const output = Buffer.from('{"choices":[{"message":{"role":"assistant","content":"done"}}]}');
  const setup = await fixture(context, (request, response) => {
    response.writeHead(200, { 'content-type': 'application/json' }); response.end(output);
  });
  const app = await createServer({ host: '127.0.0.1', port: 0, serverToken: 'local-test' }, { provider: setup.backend });
  context.after(() => app.close());
  const address = await app.listen();
  const url = `http://127.0.0.1:${address.port}`;
  const payload = { ...body, stream: false, messages: [...body.messages,
    { role: 'assistant', tool_calls: [{ id: 'call-1', type: 'function', function: { name: 'lookup', arguments: '{}' } }] },
    { role: 'tool', tool_call_id: 'call-1', content: '{"value":42}' }] };
  const denied = await fetch(url + '/v1/chat/completions', { method: 'POST', body: JSON.stringify(payload) });
  assert.equal(denied.status, 401); await denied.arrayBuffer();
  assert.equal(setup.requests.length, 0);
  const result = await fetch(url + '/v1/chat/completions', { method: 'POST', body: JSON.stringify(payload),
    headers: { authorization: 'Bearer local-test', 'content-type': 'application/json' } });
  assert.equal(result.status, 200);
  assert.deepEqual(Buffer.from(await result.arrayBuffer()), output);
  assert.deepEqual(JSON.parse(setup.requests[1].bytes).request, payload);
});

for (const action of ['return', 'abort', 'close']) {
  test(`${action} closes an unread streaming response and removes the caller listener`, { timeout: 5000 }, async context => {
    const closed = deferred();
    const setup = await fixture(context, (request, response) => {
      response.on('close', closed.resolve);
      response.writeHead(200, { 'content-type': 'text/event-stream' }); response.write('data: {}\n\n');
    });
    const controller = new AbortController();
    const result = await setup.backend.forward(call(body, controller.signal));
    if (action === 'return') await result.body.return();
    else if (action === 'close') await setup.backend.close();
    else controller.abort();
    await closed.promise;
    await setup.backend.close();
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    assert.equal(setup.closed(), 1);
  });
}

test('an unavailable credential provider reports unavailable without exposing its error', async context => {
  const setup = await fixture(context, undefined, { auth: {
    accessToken() { throw new Error('synthetic-private-credential'); }, async close() {},
  } });
  assert.deepEqual(await setup.backend.ready({ signal: signal() }), { ready: false, reason: 'credentials_unavailable' });
  await assert.rejects(setup.backend.forward(call()), { code: 'credentials_unavailable', message: 'Framer provider unavailable' });
  assert.equal(setup.requests.length, 0);
  assert.equal(setup.mints(), 0);
});

for (const action of ['abort', 'close', 'deadline']) {
  test(`${action} cancels an active mint before returning`, { timeout: 5000 }, async context => {
    const entered = deferred();
    let cleaned = false;
    const castle = { mint({ signal }) {
      entered.resolve();
      return new Promise((resolve, reject) => signal.addEventListener('abort', () => {
        cleaned = true; reject(new Error('synthetic-private-mint-error'));
      }, { once: true }));
    }, async close() {} };
    const setup = await fixture(context, undefined, { castle, headersTimeoutMs: action === 'deadline' ? 150 : 3000 });
    const controller = new AbortController();
    const checked = assert.rejects(setup.backend.forward(call(body, controller.signal)),
      { code: action === 'close' ? 'provider_closed' : action === 'deadline' ? 'headers_timeout' : 'request_cancelled' });
    await entered.promise;
    if (action === 'abort') controller.abort();
    if (action === 'close') await setup.backend.close();
    await checked;
    assert.equal(cleaned, true);
    assert.equal(setup.requests.length, 1);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  });
}

for (const input of [null, {}, { get projectId() { throw new Error('synthetic-private'); } },
  { projectId: 'bad/project' }, { projectId: 'project', origin: 'http://example.test' },
  { projectId: 'project', origin: 'https://user:secret@example.test' }]) {
  test('invalid backend configuration has a fixed safe diagnostic', () => {
    assert.throws(() => createFramerBackend(input), { message: 'Invalid Framer provider configuration' });
  });
}
