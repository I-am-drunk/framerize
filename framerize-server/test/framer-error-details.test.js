import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { once, getEventListeners } from 'node:events';
import { gzipSync } from 'node:zlib';
import { pathToFileURL } from 'node:url';

const serverRoot = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ? pathToFileURL(`${process.env.FRAMERIZE_SERVER_PACKAGE_ROOT}/`) : new URL('../', import.meta.url);
const remoteRoot = process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ? pathToFileURL(`${process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT}/`) : new URL('../../framerize-remote/', import.meta.url);
const { createFramerBackend } = await import(new URL('dist/framer-backend.js', serverRoot));
const { createFramerAuth } = await import(new URL('dist/framer-auth.js', serverRoot));
const { createServer } = await import(new URL('dist/server.js', serverRoot));
const { createInstance } = await import(new URL('dist/core.js', serverRoot));
const { createRemoteBackend } = await import(new URL('dist/index.js', remoteRoot));

async function collect(body) {
  const chunks = [];
  for await (const chunk of body) chunks.push(chunk);
  return Buffer.concat(chunks);
}

function call(protocol, stream, signal) {
  return { path: route(protocol), signal, headers: {},
    body: (async function* () { yield Buffer.from(JSON.stringify(input(protocol, stream))); })() };
}

for (const protocol of ['responses', 'messages']) for (const stream of [false, true]) {
  test(`${protocol} stream=${stream} rejects an oversized error`, { timeout: 7000 }, async context => {
    const setup = await fixture(context, (request, response) => {
      response.writeHead(429, { 'content-type': 'text/plain' }); response.end(Buffer.alloc(65_537, 97));
    });
    const controller = new AbortController();
    const pending = setup.backend.forward(call(protocol, stream, controller.signal));
    if (!stream) await assert.rejects(pending, { code: 'upstream_response_invalid', retryable: false });
    else {
      const values = events(await collect((await pending).body));
      assert.equal(values.length, 1);
      const error = protocol === 'messages' ? values[0].error : values[0];
      assert.equal(error.code, 'upstream_response_invalid');
    }
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    assert.equal(setup.mints(), 1);
  });
}

function input(protocol, stream) {
  return { model: 'synthetic-model', stream, temperature: 0.25,
    ...(protocol === 'responses' ? { input: 'hello' }
      : { messages: [{ role: 'user', content: 'hello' }], max_tokens: 32 }) };
}

for (const protocol of ['responses', 'messages']) for (const stream of [false, true]) {
  test(`${protocol} stream=${stream} cancels a partial error`, { timeout: 7000 }, async context => {
    const entered = Promise.withResolvers();
    const closed = Promise.withResolvers();
    const setup = await fixture(context, (request, response) => {
      response.once('close', closed.resolve);
      response.writeHead(429, { 'content-type': 'application/json' });
      response.write('{"error":');
      entered.resolve();
    });
    const controller = new AbortController();
    context.after(() => controller.abort());
    const outcome = setup.backend.forward(call(protocol, stream, controller.signal))
      .then(async result => collect(result.body)).then(() => null, error => error);
    await entered.promise;
    controller.abort();
    assert.equal((await outcome).code, 'request_cancelled');
    await closed.promise;
    await setup.backend.close();
    assert.equal(setup.closes(), 1);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  });
}

const route = protocol => protocol === 'responses' ? '/openai/v1/responses' : '/anthropic/v1/messages';

const upstream = { error: { type: 'quota_error', code: 'quota_exhausted',
  message: 'Synthetic quota detail: 雪\nretry later', param: 'model',
  details: { balance: 0, reset: 123 }, sequence_number: 77 }, request_id: 'synthetic-request' };
const json = Buffer.from(` ${JSON.stringify(upstream)}\n`);
const success = protocol => protocol === 'responses'
  ? { id: 'resp', object: 'response', status: 'completed', model: 'synthetic-model', created_at: 1,
    output: [{ type: 'message', id: 'out', role: 'assistant', status: 'completed',
      content: [{ type: 'output_text', text: 'hello' }] }] }
  : { id: 'msg', type: 'message', role: 'assistant', model: 'synthetic-model',
    stop_reason: 'end_turn', stop_sequence: null, content: [{ type: 'text', text: 'hello' }],
    usage: { input_tokens: 1, output_tokens: 1 } };
const text = 'Synthetic plain failure: 雪\nevent: not-an-event\ndata: not-an-event';
const representations = [
  { name: 'JSON', status: 400, bytes: json, type: 'application/problem+json', value: upstream },
  { name: 'gzip JSON', status: 502, bytes: gzipSync(json), type: 'application/json', encoding: 'gzip', value: upstream },
  { name: 'text', bytes: Buffer.from(text), type: 'text/plain; charset=utf-8', value: text },
  { name: 'opaque', status: 403, bytes: Buffer.from([0xff, 0xfe, 0, 0x80]), type: 'application/octet-stream', opaque: true },
  { name: 'invalid gzip', status: 500, bytes: Buffer.from('not gzip'), type: 'application/json', encoding: 'gzip', opaque: true },
  { name: 'expanded gzip limit', bytes: gzipSync(Buffer.alloc(65_537, 97)),
    type: 'text/plain', encoding: 'gzip', opaque: true },
  { name: 'wire limit', bytes: Buffer.alloc(65_536, 97), type: 'text/plain', value: 'a'.repeat(65_536) },
];

function assertError(received, protocol, sample) {
  assert.equal(received.type, 'error');
  const error = protocol === 'messages' ? received.error : received;
  if (sample.opaque) {
    assert.deepEqual(received.upstream, { encoding: 'base64',
      content_encoding: sample.encoding ?? 'identity', data: sample.bytes.toString('base64') });
  } else {
    assert.deepEqual(received.upstream, sample.value);
    if (typeof sample.value === 'string') assert.equal(error.message, sample.value);
    else {
      assert.equal(error.code, upstream.error.code);
      assert.equal(error.message, upstream.error.message);
      assert.deepEqual(error.details, upstream.error.details);
      assert.equal(error.param, 'model');
      if (protocol === 'messages') assert.equal(error.type, 'quota_error');
    }
  }
  if (protocol === 'responses') assert.equal(received.sequence_number, 0);
}

async function fixture(context, handler, overrides = {}) {
  const requests = [];
  let mints = 0;
  let closes = 0;
  let backend;
  const apps = [];
  const gateway = http.createServer(async (request, response) => {
    const bytes = await collect(request);
    if (request.url === '/auth/ai/access-token') {
      response.setHeader('content-type', 'application/json');
      response.end(JSON.stringify({ accessToken: 'synthetic-access',
        expiresAt: new Date(Date.now() + 600_000).toISOString() }));
      return;
    }
    requests.push({ path: request.url, headers: request.headers, value: JSON.parse(bytes) });
    handler(request, response);
  });
  context.after(async () => {
    for (const app of apps.reverse()) await app.close();
    await backend?.close();
    gateway.closeAllConnections();
    await new Promise(resolve => gateway.close(resolve));
  });
  gateway.listen(0, '127.0.0.1');
  await once(gateway, 'listening');
  const origin = `http://127.0.0.1:${gateway.address().port}`;
  backend = createFramerBackend({ projectId: 'synthetic-project', origin,
    auth: createFramerAuth({ sessionCookie: 'session=synthetic-cookie', origin }),
    castle: { async mint({ signal }) {
      signal.throwIfAborted(); mints += 1; return { token: 'synthetic-token' };
    }, async close() { closes += 1; } },
    headersTimeoutMs: 2000, inferenceTimeoutMs: 3000, idleTimeoutMs: 1000, ...overrides });
  return { backend, requests, apps, mints: () => mints, closes: () => closes };
}

async function listener(setup, via) {
  const app = await createServer({ host: '127.0.0.1', port: 0, serverToken: 'synthetic-server' },
    { provider: setup.backend });
  setup.apps.push(app);
  const address = await app.listen();
  if (via === 'server') return { port: address.port, token: 'synthetic-server' };
  const backend = createRemoteBackend({ endpoint: { origin: `http://127.0.0.1:${address.port}` },
    serverToken: 'synthetic-server', headersTimeoutMs: 2000, inferenceTimeoutMs: 4000,
    idleTimeoutMs: 2000, readinessBodyMaxBytes: 4096, readinessWholeMs: 2000 });
  const remote = createInstance({ backend, token: 'synthetic-local', host: '127.0.0.1', port: 0 });
  setup.apps.push(remote);
  return { port: (await remote.listen()).port, token: 'synthetic-local' };
}

async function request(context, target, protocol, stream) {
  const outgoing = http.request({ host: '127.0.0.1', port: target.port, path: route(protocol),
    method: 'POST', agent: false, signal: context.signal,
    headers: { authorization: `Bearer ${target.token}`, 'content-type': 'application/json' } });
  const pending = once(outgoing, 'response');
  outgoing.end(JSON.stringify(input(protocol, stream)));
  const [response] = await pending;
  return { status: response.statusCode, headers: response.headers, bytes: await collect(response) };
}

function events(bytes) {
  return bytes.toString().split('\n\n').flatMap(frame => {
    const data = frame.split('\n').find(line => line.startsWith('data: '));
    return data ? [JSON.parse(data.slice(6))] : [];
  });
}

for (const protocol of ['responses', 'messages']) for (const via of ['server', 'remote']) {
  for (const stream of [false, true]) {
    test(`${via} ${protocol} stream=${stream} success is unchanged`, { timeout: 7000 }, async context => {
      const bytes = Buffer.from(JSON.stringify(success(protocol)));
      const setup = await fixture(context, (request, response) => {
        response.writeHead(200, { 'content-type': 'application/json' }); response.end(bytes);
      });
      const result = await request(context, await listener(setup, via), protocol, stream);
      assert.equal(result.status, 200);
      if (!stream) assert.deepEqual(result.bytes, bytes);
      else {
        const values = events(result.bytes);
        assert.equal(values.at(-1).type, protocol === 'responses' ? 'response.completed' : 'message_stop');
        assert.ok(values.every(value => value.type !== 'error'));
      }
      assert.equal(setup.mints(), 1);
      assert.equal(setup.requests.length, 1);
      assert.deepEqual(setup.requests[0].value.request, input(protocol, false));
    });
  }
  for (const stream of [false, true]) for (const sample of representations) {
    test(`${via} ${protocol} stream=${stream} preserves ${sample.name} errors`, { timeout: 7000 }, async context => {
      const headers = { 'content-type': sample.type, 'cache-control': 'no-store',
        'x-request-id': 'synthetic-upstream', 'retry-after': '7', 'x-should-retry': 'true',
        ...(sample.encoding ? { 'content-encoding': sample.encoding } : {}) };
      const setup = await fixture(context, (request, response) => {
        response.writeHead(sample.status ?? 429, headers);
        response.end(sample.bytes);
      });
      const result = await request(context, await listener(setup, via), protocol, stream);
      assert.equal(setup.requests.length, 1);
      assert.equal(setup.mints(), 1);
      assert.equal(setup.requests[0].path, `/ai/v3/${protocol}/`);
      assert.deepEqual(setup.requests[0].value.request, input(protocol, false));
      assert.equal(setup.requests[0].headers.cookie, undefined);
      assert.equal(setup.requests[0].headers.authorization, 'Bearer synthetic-access');
      assert.equal(result.headers['x-should-retry'], 'false');
      if (stream) {
        assert.equal(result.status, 200);
        assert.match(result.headers['content-type'], /^text\/event-stream/);
        const values = events(result.bytes);
        assert.equal(values.length, 1);
        assertError(values[0], protocol, sample);
      } else {
        assert.equal(result.status, sample.status ?? 429);
        for (const name of ['content-type', 'content-encoding', 'cache-control', 'retry-after']) {
          assert.equal(result.headers[name], headers[name]);
        }
        assert.deepEqual(result.bytes, sample.bytes);
      }
    });
  }
}
