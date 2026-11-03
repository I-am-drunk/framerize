import assert from 'node:assert/strict';
import test from 'node:test';
import { createInstance } from '../src/instance.js';
import type { BackendReadiness } from '../src/backend.js';
import type { InstanceOptions } from '../src/instance-options.js';
import { authorization, backend, body, comparable, exchange, rawExchange, token,
  unavailable } from './http-fixtures.js';
import { acceptedResponses } from './accepted-responses.js';

test('readiness is fresh, authenticated, and bounded', async () => {
  let calls = 0;
  let result: BackendReadiness = true;
  let stalled = false;
  let signal: AbortSignal | undefined;
  const instance = createInstance({ token, headersTimeoutMs: 50, backend: backend({
    async ready(context) {
      calls += 1;
      signal = context.signal;
      return stalled ? new Promise(() => {}) : result;
    },
  }) });
  try {
    const { port } = await instance.listen();
    assert.equal((await exchange(port, '/ready')).status, 401);
    assert.equal(calls, 0);
    assert.equal((await exchange(port, '/ready', { headers: authorization })).status, 200);
    result = { ready: false, reason: 'credentials_unavailable' };
    const unavailable = await exchange(port, '/ready', { headers: authorization });
    assert.equal(JSON.parse(unavailable.body.toString()).reason, 'credentials_unavailable');
    stalled = true;
    assert.equal((await exchange(port, '/ready', { headers: authorization })).status, 503);
    assert.equal(signal?.aborted, true);
    assert.equal(calls, 3);
  } finally { await instance.close(); }
});

test('keeps all bounded header pairs for late nominations and duplicate auth', async () => {
  const forwarded: Record<string, string>[] = [];
  const instance = createInstance({ token, backend: backend({
    async forward(request) {
      forwarded.push({ ...request.headers });
      return { status: 200, headers: { 'X-Request-Id': 'hidden-response',
        Connection: 'X-Request-Id', 'Content-Type': 'application/json' }, body: body() };
    },
  }) });
  try {
    const { port } = await instance.listen();
    const prefix = `POST /v1/responses HTTP/1.1\r\nHost: local\r\nAuthorization: Bearer ${token}\r\n`;
    for (const fillers of [0, 2100]) {
      const response = await rawExchange(port, prefix + 'Accept: hidden-request\r\n'
        + 'X:a\r\n'.repeat(fillers) + 'Connection: Accept, close\r\nContent-Length: 2\r\n\r\n{}');
      assert.match(response, /^HTTP\/1.1 200/);
      assert.doesNotMatch(response, /hidden-response/i);
      assert.equal(forwarded.at(-1)?.accept, undefined);
    }
    const duplicate = await rawExchange(port, prefix + 'X:a\r\n'.repeat(2100)
      + `Authorization: Bearer ${token}\r\nConnection: close\r\nContent-Length: 2\r\n\r\n{}`);
    assert.match(duplicate, /^HTTP\/1.1 401/);
    const oversized = await rawExchange(port, prefix + `X: ${'a'.repeat(17000)}\r\n\r\n`);
    assert.match(oversized, /^HTTP\/1.1 431/);
    assert.equal(forwarded.length, 2);
  } finally { await instance.close(); }
});

test('configuration errors are constant and no listener starts implicitly', async () => {
  for (const change of [{ token: '' }, { token: 'bad secret' }, { port: -1 },
    { port: 65536 }, { idleTimeoutMs: 0 }, { headersTimeoutMs: Infinity },
    { maxBodyBytes: NaN }, { shutdownTimeoutMs: 2147483648 }, { host: '/private/secret' }]) {
    assert.throws(() => createInstance({ token, backend: backend(), ...change }),
      { message: 'Invalid instance configuration' });
  }
  const hostile = new Proxy({}, { ownKeys() { throw new Error('hidden-options'); } });
  assert.throws(() => createInstance(hostile as InstanceOptions),
    { message: 'Invalid instance configuration' });
  let closes = 0;
  const instance = createInstance({ token, backend: backend({ close() { closes += 1; } }) });
  const first = instance.close();
  assert.equal(instance.close(), first);
  await first;
  assert.equal(closes, 1);
  assert.equal(instance.closing, true);
  await assert.rejects(instance.listen(), { message: 'Server is closing' });
});

test('backend errors and request events exclude secret text', async () => {
  const events: unknown[] = [];
  const instance = createInstance({ token, onRequest(event) { events.push(event); },
    backend: backend({ async forward() { throw new Error('hidden-backend-secret'); } }) });
  try {
    const { port } = await instance.listen();
    const failure = await exchange(port, '/v1/responses?hidden-query-secret',
      { method: 'POST', headers: authorization });
    assert.equal(failure.status, 502);
    assert.equal(JSON.parse(failure.body.toString()).error.message, 'Upstream request failed');
    assert.doesNotMatch(JSON.stringify(events), /hidden|synthetic-local-token/);
    assert.equal(events.length, 1);
  } finally { await instance.close(); }
});

test('throwing error accessors and an async logger cannot escape the request boundary', async () => {
  const instance = createInstance({ token,
    async onRequest() { throw new Error('hidden-logger'); },
    backend: backend({ async forward() {
      throw new Proxy({}, { get() { throw new Error('hidden-getter'); } });
    } }) });
  try {
    const { port } = await instance.listen();
    const reply = await exchange(port, '/v1/responses', { method: 'POST', headers: authorization });
    assert.equal(reply.status, 502);
    assert.equal(JSON.parse(reply.body.toString()).error.message, 'Upstream request failed');
    assert.doesNotMatch(reply.body.toString(), /hidden/);
  } finally { await instance.close(); }
});

test('bind failure is constant and closes the backend exactly once', async () => {
  const first = createInstance({ token, backend: backend() });
  let closes = 0;
  const { port } = await first.listen();
  const second = createInstance({ port, token, backend: backend({ close() { closes += 1; } }) });
  try {
    await assert.rejects(second.listen(), { message: 'Instance listen failed' });
    await second.close();
    await second.close();
    assert.equal(closes, 1);
  } finally { await Promise.all([first.close(), second.close()]); }
  await unavailable(port);
});

test('known backend failures preserve accepted status and constant diagnostics', async () => {
  for (const code of ['credentials_unavailable', 'provider_closed', 'upstream_error', 'unrecognized']) {
    const provider = backend({ async forward() {
      throw Object.assign(new Error('hidden-message'), { code, status: 503 });
    }, async ready() { throw { code: 'credentials_unavailable' }; } });
    const direct = createInstance({ token, backend: provider });
    try {
      const local = await direct.listen();
      for (const path of ['/v1/responses', '/ready']) {
        const options = { method: path === '/ready' ? 'GET' : 'POST', headers: authorization };
        assert.deepEqual(comparable(await exchange(local.port, path, options)),
          acceptedResponses.get(`${code} ${path}`));
      }
    } finally { await direct.close(); }
  }
});

test('shared instance preserves 20 captured HTTP responses over an actual listener', async () => {
  const direct = createInstance({ token, backend: backend() });
  try {
    const local = await direct.listen();
    for (const path of ['/health', '/ready', '/missing', '/v1/responses', '/v1/chat/completions']) {
      for (const method of ['GET', 'POST']) {
        for (const headers of [{}, authorization]) {
          const actual = await exchange(local.port, path, { method, headers });
          const expected = acceptedResponses.get(`${method} ${path} ${headers === authorization}`);
          assert.deepEqual(comparable(actual), expected, `${method} ${path}`);
        }
      }
    }
  } finally { await direct.close(); }
});

test('rejects invalid bodies before invoking the backend and preserves JSON bytes', async () => {
  const seen: Buffer[] = [];
  const instance = createInstance({ token, maxBodyBytes: 40, backend: backend({
    async forward(request) {
      for await (const chunk of request.body) seen.push(chunk);
      return { status: 200, headers: {}, body: body() };
    },
  }) });
  try {
    const { port } = await instance.listen();
    for (const value of ['', '[]', 'null', '1', '{', '"text"']) {
      assert.equal((await exchange(port, '/v1/responses', { method: 'POST',
        headers: authorization }, Buffer.from(value))).status, 400);
    }
    assert.equal((await exchange(port, '/v1/responses', { method: 'POST',
      headers: authorization }, Buffer.alloc(41, 32))).status, 413);
    assert.equal((await exchange(port, '/v1/responses', { method: 'POST',
      headers: authorization }, Buffer.from([0xff]))).status, 400);
    assert.equal(seen.length, 0);
    const original = Buffer.from('{ "model" : "synthetic", "stream":true }');
    assert.equal((await exchange(port, '/v1/responses', { method: 'POST',
      headers: authorization }, original)).status, 200);
    assert.deepEqual(seen, [original]);
  } finally { await instance.close(); }
});
