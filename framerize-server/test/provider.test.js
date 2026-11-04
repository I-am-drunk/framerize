import { test as nodeTest } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import net from 'node:net';
import { once, getEventListeners } from 'node:events';
import { Readable } from 'node:stream';
import { setTimeout as delay } from 'node:timers/promises';
import { gzipSync } from 'node:zlib';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { inspect, promisify } from 'node:util';
import { execFile } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { createProvider } from '../dist/provider.js';
import { ConfigurationError, ProviderError } from '../dist/errors.js';

const test = (name, run) => nodeTest(name, { timeout: 5_000 }, run);
const body = (value = '{}') => Readable.from([Buffer.from(value)]);
const collect = async (stream) => {
  const chunks = [];
  for await (const chunk of stream) { assert.ok(Buffer.isBuffer(chunk)); chunks.push(chunk); }
  return Buffer.concat(chunks);
};
const deferred = () => {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
};

test('non-streaming inference outlives connection and idle limits', async context => {
  const { provider } = await fixture(context, (request, response) => {
    request.resume();
    const timer = setTimeout(() => response.end('{"done":true}'), 160);
    response.once('close', () => clearTimeout(timer));
  }, { headersTimeoutMs: 40, idleTimeoutMs: 40, inferenceTimeoutMs: 1000 });
  const result = await provider.forward({ path: '/v1/responses', body: body('{"stream":false}') });
  assert.equal(result.status, 200);
  assert.equal((await collect(result.body)).toString(), '{"done":true}');
});
async function fixture(context, handler, config = {}) {
  const errors = [];
  const server = http.createServer((request, response) => {
    Promise.resolve(handler(request, response)).catch((error) => { errors.push(error); response.destroy(); });
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const upstream = `http://127.0.0.1:${server.address().port}`;
  const provider = createProvider({ upstream, ...config });
  context.after(async () => {
    await provider.close(); server.closeAllConnections(); await new Promise((done) => server.close(done));
    assert.deepEqual(errors, []);
  });
  return { provider, server, upstream };
}

for (const status of [200, 502]) {
  for (const scenario of [
    { name: 'false', value: 'false', expected: 'false' },
    { name: 'true', value: 'true' },
    { name: 'unknown', value: 'unknown' },
    { name: 'duplicates', value: ['false', 'false'] },
    { name: 'nominated', value: 'false', connection: 'X-Should-Retry' },
  ]) {
    test(`retry header ${status}: ${scenario.name}`, async context => {
      const { provider } = await fixture(context, (request, response) => {
        request.resume();
        response.writeHead(status, { 'x-should-retry': scenario.value,
          ...(scenario.connection ? { connection: scenario.connection } : {}) });
        response.end('synthetic-private-upstream');
      });
      const result = await provider.forward({ path: '/v1/responses', body: body() });
      assert.equal(result.status, status);
      assert.equal(result.headers['x-should-retry'], scenario.expected);
      const bytes = await collect(result.body);
      if (status === 502) assert.doesNotMatch(bytes.toString(), /synthetic-private-upstream/);
    });
  }
}

async function rawFixture(context, response) {
  const sockets = new Set(); const closed = deferred(); const logs = [];
  const controller = new AbortController();
  let calls = 0;
  const server = net.createServer((socket) => {
    sockets.add(socket);
    socket.on('error', () => {});
    socket.once('close', () => { sockets.delete(socket); closed.resolve(); });
    socket.once('data', () => { calls += 1; socket.write(response); });
  });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  const provider = createProvider({ upstream: `http://127.0.0.1:${server.address().port}`,
    logger: { info: (...args) => logs.push(args), error: (...args) => logs.push(args) } });
  context.after(async () => {
    await provider.close();
    for (const socket of sockets) socket.destroy();
    await new Promise((done) => server.close(done));
  });
  return { provider, signal: controller.signal, async checkClosed() {
    await closed.promise;
    assert.equal(calls, 1);
    assert.equal(sockets.size, 0);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    assert.deepEqual(logs, []);
  } };
}

for (const path of ['/v1/responses', '/v1/chat/completions']) {
  test(`${path} preserves short streaming bytes and safe headers`, async (context) => {
    const expected = Buffer.from('data: {"delta":"hello 😀"}\n\ndata: [DONE]\n\n');
    const { provider } = await fixture(context, async (request, response) => {
      assert.equal(request.url, path);
      assert.equal(request.method, 'POST');
      assert.equal((await collect(request)).toString(), '{"stream":true}');
      response.writeHead(201, { 'content-type': 'text/event-stream', 'x-request-id': 'fixture-id',
        'set-cookie': 'private=yes', 'x-private': 'never-forward', connection: 'close' });
      response.end(expected);
    });
    const result = await provider.forward({ path, body: body('{"stream":true}') });
    assert.equal(result.status, 201);
    assert.deepEqual(result.headers, { 'content-type': 'text/event-stream', 'x-request-id': 'fixture-id' });
    assert.deepEqual(await collect(result.body), expected);
  });
}

for (const status of [200, 429]) for (const fillerCount of [0, 2100]) {
  test(`response headers honor late Connection with ${fillerCount} fields at status ${status}`, async (context) => {
    const payload = status === 200 ? 'data: {"delta":"fixture"}\n\n' : '{"error":"FAKE_BODY_SECRET"}';
    const headers = `HTTP/1.1 ${status} Fixture\r\nX-Request-Id: nominated-canary\r\n`
      + 'Retry-After: 17\r\nContent-Type: text/event-stream\r\nX-Private: FAKE_HEADER_SECRET\r\n'
      + 'X:a\r\n'.repeat(fillerCount) + 'Connection: X-Request-Id, close\r\n'
      + `Content-Length: ${Buffer.byteLength(payload)}\r\n\r\n`;
    assert.ok(Buffer.byteLength(headers) < 16_384);
    const { provider, signal, checkClosed } = await rawFixture(context, headers + payload);
    const result = await provider.forward({ path: '/v1/responses', body: body(), signal });
    assert.equal(result.status, status);
    assert.equal(result.headers['x-request-id'], undefined);
    assert.equal(result.headers['retry-after'], '17');
    assert.equal(result.headers['x-private'], undefined);
    const received = await collect(result.body);
    if (status === 200) assert.deepEqual(received, Buffer.from(payload));
    else {
      assert.equal(result.headers['content-type'], 'application/json');
      assert.deepEqual(JSON.parse(received), { error: { type: 'upstream_error',
        code: 'upstream_http_error', message: 'Upstream request failed' } });
    }
    assert.doesNotMatch(inspect(result.headers) + received, /FAKE_.*_SECRET|nominated-canary/);
    await checkClosed();
  });
}

for (const status of [200, 429]) for (const kind of ['single', 'repeated']) {
  test(`response header byte cap rejects ${kind} fields at status ${status}`, async (context) => {
    context.diagnostic(`global header limit ${http.maxHeaderSize}`);
    const field = 'X-Secret: FAKE_HEADER_SECRET';
    const fields = kind === 'single' ? field + 'a'.repeat(17_000) + '\r\n'
      : field + '\r\n' + 'X:a\r\n'.repeat(9000);
    const payload = 'FAKE_BODY_SECRET';
    const headers = `HTTP/1.1 ${status} Fixture\r\n${fields}Connection: keep-alive\r\n`
      + `Content-Length: ${payload.length}\r\n\r\n`;
    assert.ok(Buffer.byteLength(headers) > 16_384 && Buffer.byteLength(headers) < 65_536);
    const { provider, signal, checkClosed } = await rawFixture(context, headers + payload);
    await assert.rejects(provider.forward({ path: '/v1/responses', body: body(), signal }), (error) => {
      assert.ok(error instanceof ProviderError);
      assert.equal(error.code, 'upstream_error');
      assert.equal(error.status, 502);
      assert.equal(error.message, 'Upstream request failed');
      assert.equal(error.cause, undefined);
      assert.doesNotMatch(inspect(error) + JSON.stringify(error), /FAKE_.*_SECRET|X-Secret/);
      return true;
    });
    await checkClosed();
  });
}

test('response header global cap override does not weaken the byte limit', async (context) => {
  const directory = await mkdtemp(join(tmpdir(), 'provider-header-cap-'));
  context.after(() => rm(directory, { recursive: true, force: true }));
  const env = { PATH: dirname(process.execPath), HOME: directory, USERPROFILE: directory,
    XDG_CONFIG_HOME: directory, APPDATA: directory, LOCALAPPDATA: directory,
    TMPDIR: directory, TMP: directory, TEMP: directory, NODE_OPTIONS: process.env.NODE_OPTIONS ?? '' };
  if (process.platform === 'win32') env.SystemRoot = process.env.SystemRoot;
  const { stdout } = await promisify(execFile)(process.execPath, [
    '--max-http-header-size=65536', '--test', '--test-reporter=tap',
    '--test-name-pattern=^response header byte cap', fileURLToPath(import.meta.url),
  ], { env, timeout: 4_000, maxBuffer: 1024 * 1024 });
  assert.match(stdout, /global header limit 65536/);
  for (const status of [200, 429]) for (const kind of ['single', 'repeated']) {
    assert.match(stdout, new RegExp(`ok \\d+ - response header byte cap rejects ${kind} fields at status ${status}\\r?\\n`));
  }
});

test('delayed iteration retains every byte without starting flowing mode', async (context) => {
  const expected = Buffer.from('firstsecond');
  const { provider } = await fixture(context, (request, response) => {
    request.resume();
    response.write('first'); response.end('second');
  }, { idleTimeoutMs: 30 });
  const result = await provider.forward({ path: '/v1/responses', body: body() });
  await delay(90);
  assert.deepEqual(await collect(result.body), expected);
});

test('fragmented UTF-8, tool events and binary bytes are passed through exactly', async (context) => {
  const expected = Buffer.concat([Buffer.from('event: response.function_call_arguments.delta\n'
    + 'data: {"delta":"😀é工具"}\n\ndata: [DONE]\n\n'), Buffer.from([0, 255, 128])]);
  const { provider } = await fixture(context, async (request, response) => {
    assert.deepEqual(await collect(request), expected);
    for (const byte of expected) { response.write(Buffer.from([byte])); await delay(1); }
    response.end();
  });
  const chunks = Readable.from([...expected].map((byte) => Buffer.from([byte])));
  const result = await provider.forward({ path: '/v1/responses', body: chunks });
  assert.deepEqual(await collect(result.body), expected);
});

test('gzip success preserves both the compressed body and its encoding', async (context) => {
  const compressed = gzipSync(Buffer.from('{"output":"hello"}'));
  const { provider } = await fixture(context, (request, response) => {
    request.resume();
    response.writeHead(200, { 'content-type': 'application/json', 'content-encoding': 'gzip' });
    response.end(compressed);
  });
  const result = await provider.forward({ path: '/v1/responses', body: body() });
  assert.equal(result.headers['content-encoding'], 'gzip');
  assert.deepEqual(await collect(result.body), compressed);
});

test('active body outlives the cleared header deadline', async (context) => {
  const { provider } = await fixture(context, async (request, response) => {
    request.resume(); response.flushHeaders();
    for (let index = 0; index < 12; index += 1) { response.write('chunk'); await delay(25); }
    response.end('[DONE]');
  }, { inferenceTimeoutMs: 70, idleTimeoutMs: 100 });
  const result = await provider.forward({ path: '/v1/responses', body: body() });
  const started = performance.now();
  assert.equal((await collect(result.body)).toString(), 'chunk'.repeat(12) + '[DONE]');
  assert.ok(performance.now() - started > 200);
});

for (const operation of ['forward', 'ready']) {
  test(`${operation}: queued headers survive an event-loop stall past the header deadline`, async (context) => {
    const { provider } = await fixture(context, async (request, response) => {
      await collect(request);
      setImmediate(() => {
        response.write('fixture-', (error) => {
          assert.ifError(error);
          Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 120);
          const completion = setTimeout(() => response.end('ok'), 70);
          response.once('close', () => clearTimeout(completion));
        });
      });
    }, { headersTimeoutMs: 40, inferenceTimeoutMs: 40 });
    if (operation === 'ready') assert.equal(await provider.ready(), true);
    else {
      const result = await provider.forward({ path: '/v1/responses', body: body() });
      assert.equal(result.status, 200);
      assert.equal((await collect(result.body)).toString(), 'fixture-ok');
    }
  });
}

test('slow consumers bound response buffering and can pause past the idle interval', async (context) => {
  const total = 512;
  const chunk = Buffer.alloc(65_536, 7);
  let writes = 0;
  const { provider } = await fixture(context, async (request, response) => {
    request.resume();
    for (; writes < total; writes += 1) if (!response.write(chunk)) await once(response, 'drain');
    response.end();
  }, { idleTimeoutMs: 40 });
  const result = await provider.forward({ path: '/v1/responses', body: body() });
  await delay(120);
  assert.ok(writes < total, `upstream was eagerly drained: ${writes}`);
  let bytes = 0;
  for await (const received of result.body) {
    assert.ok(received.every((value) => value === 7));
    bytes += received.length;
    if (bytes === received.length) await delay(120);
  }
  assert.equal(bytes, total * chunk.length);
});

test('resuming after backpressure reads queued data before declaring an idle timeout', async (context) => {
  let upstream;
  const { provider } = await fixture(context, (request, response) => {
    request.resume(); upstream = response; response.write('first');
  }, { idleTimeoutMs: 40 });
  const result = await provider.forward({ path: '/v1/responses', body: body() });
  assert.equal((await result.body.next()).value.toString(), 'first');
  await delay(120);
  const pending = result.body.next();
  setImmediate(() => {
    upstream.end('second');
    Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 100);
  });
  assert.equal((await pending).value.toString(), 'second');
  assert.equal((await result.body.next()).done, true);
});

test('request bodies reach upstream incrementally with bounded backpressure', async (context) => {
  const arrived = deferred(); const release = deferred();
  let produced = 0;
  const chunk = Buffer.alloc(65_536, 9);
  const { provider } = await fixture(context, async (request, response) => {
    arrived.resolve(); await release.promise;
    assert.equal((await collect(request)).length, 512 * chunk.length);
    response.end('ok');
  }, { idleTimeoutMs: 1_000 });
  async function* source() { for (; produced < 512; produced += 1) yield chunk; }
  const pending = provider.forward({ path: '/v1/responses', body: source() });
  await arrived.promise; await delay(100);
  assert.ok(produced > 0 && produced < 512, `upload was eagerly buffered: ${produced}`);
  release.resolve();
  assert.equal((await collect((await pending).body)).toString(), 'ok');
});

test('abort releases an upload waiting for drain and returns its iterator', async (context) => {
  const arrived = deferred(); const returned = deferred();
  const controller = new AbortController();
  const { provider } = await fixture(context, (request) => { request.pause(); arrived.resolve(); });
  async function* source() {
    try { while (true) yield Buffer.alloc(65_536); } finally { returned.resolve(); }
  }
  const pending = provider.forward({ path: '/v1/responses', body: source(), signal: controller.signal });
  const rejected = assert.rejects(pending, { code: 'request_cancelled', status: 499 });
  await arrived.promise; await delay(60); controller.abort(new Error('FAKE_PRIVATE_REASON'));
  await rejected; await returned.promise;
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});

test('abort interrupts a stuck request iterator and invokes return without awaiting it', async (context) => {
  const requested = deferred(); const returned = deferred();
  const controller = new AbortController();
  const { provider } = await fixture(context, () => {});
  const source = {
    next() { requested.resolve(); return new Promise(() => {}); },
    return() { returned.resolve(); return new Promise(() => {}); },
    [Symbol.asyncIterator]() { return this; },
  };
  const pending = provider.forward({ path: '/v1/responses', body: source, signal: controller.signal });
  const rejected = assert.rejects(pending, { code: 'request_cancelled' });
  await requested.promise; controller.abort(); await rejected; await returned.promise;
});

test('header silence times out without retrying', async (context) => {
  let calls = 0;
  const { provider } = await fixture(context, (request) => { calls += 1; request.resume(); },
    { inferenceTimeoutMs: 50, idleTimeoutMs: 500 });
  await assert.rejects(provider.forward({ path: '/v1/responses', body: body() }),
    { code: 'headers_timeout', status: 504 });
  assert.equal(calls, 1);
});

test('a missing first chunk and a midstream idle gap reject iteration', async (context) => {
  for (const first of ['', 'partial']) {
    const { provider } = await fixture(context, (request, response) => {
      request.resume(); response.flushHeaders(); if (first) response.write(first);
    }, { idleTimeoutMs: 40 });
    const result = await provider.forward({ path: '/v1/responses', body: body() });
    const chunks = [];
    await assert.rejects(async () => {
      for await (const chunk of result.body) chunks.push(chunk);
    }, { code: 'idle_timeout', status: 504 });
    assert.equal(Buffer.concat(chunks).toString(), first);
  }
});

test('midstream socket failure rejects without synthesizing a successful terminal event', async (context) => {
  const { provider } = await fixture(context, (request, response) => {
    request.resume(); response.write('data: partial\n\n');
    setTimeout(() => response.destroy(), 30);
  });
  const result = await provider.forward({ path: '/v1/responses', body: body() });
  const chunks = [];
  await assert.rejects(async () => {
    for await (const chunk of result.body) chunks.push(chunk);
  }, { code: 'upstream_error', status: 502 });
  assert.equal(Buffer.concat(chunks).toString(), 'data: partial\n\n');
});

test('disconnect before body consumption remains observable as a safe typed error', async (context) => {
  const { provider } = await fixture(context, (request, response) => {
    request.resume(); response.flushHeaders(); setTimeout(() => response.destroy(), 10);
  });
  const result = await provider.forward({ path: '/v1/responses', body: body() });
  await delay(50);
  await assert.rejects(collect(result.body), { code: 'upstream_error' });
});

test('non-2xx statuses redact upstream bodies and never follow redirects or retry', async (context) => {
  const statuses = [301, 307, 400, 401, 403, 429, 500, 503];
  const logs = [];
  let calls = 0;
  const { provider } = await fixture(context, (request, response) => {
    request.resume();
    response.writeHead(statuses[calls++], { 'content-type': 'application/json',
      'content-encoding': 'gzip', location: 'http://127.0.0.1:1/private',
      'retry-after': '12', 'x-request-id': 'fixture-error', 'set-cookie': 'private=value' });
    response.end(gzipSync(Buffer.from('{"error":"FAKE_UPSTREAM_SECRET"}')));
  }, { logger: { info: (...args) => logs.push(args), error: (...args) => logs.push(args) } });
  for (const status of statuses) {
    const result = await provider.forward({ path: '/v1/responses', body: body() });
    assert.equal(result.status, status);
    assert.equal(result.headers['retry-after'], '12');
    assert.equal(result.headers['x-request-id'], 'fixture-error');
    assert.equal(result.headers['content-encoding'], undefined);
    assert.equal(result.headers.location, undefined);
    const received = await collect(result.body);
    assert.deepEqual(JSON.parse(received), { error: { type: 'upstream_error',
      code: 'upstream_http_error', message: 'Upstream request failed' } });
    assert.ok(!received.includes('FAKE_UPSTREAM_SECRET'));
  }
  assert.equal(calls, statuses.length);
  assert.deepEqual(logs, []);
});

test('an endless error body is discarded only to the byte limit and its socket closes', async (context) => {
  const closed = deferred();
  const { provider } = await fixture(context, (request, response) => {
    request.resume(); response.once('close', closed.resolve);
    response.writeHead(500); response.write(Buffer.alloc(131_072, 65));
  });
  const result = await provider.forward({ path: '/v1/responses', body: body() });
  assert.equal(result.status, 500); await closed.promise;
  assert.ok(!(await collect(result.body)).includes('AAAA'));
});

test('an idle upstream error body still returns the sanitized HTTP status', async (context) => {
  const { provider } = await fixture(context, (request, response) => {
    request.resume(); response.writeHead(403); response.flushHeaders();
  }, { idleTimeoutMs: 30 });
  const result = await provider.forward({ path: '/v1/responses', body: body() });
  assert.equal(result.status, 403);
  assert.equal(JSON.parse(await collect(result.body)).error.message, 'Upstream request failed');
});

test('private credential replaces caller secrets and connection-nominated headers are dropped', async (context) => {
  const { provider } = await fixture(context, async (request, response) => {
    assert.equal(request.headers.authorization, 'Bearer PRIVATE_FIXTURE_KEY');
    assert.equal(request.headers.cookie, undefined);
    assert.equal(request.headers['x-api-key'], undefined);
    assert.equal(request.headers['x-forwarded-for'], undefined);
    assert.equal(request.headers['openai-beta'], undefined);
    assert.equal(request.headers.accept, 'application/json');
    assert.equal(request.headers['content-type'], 'application/json');
    await collect(request);
    response.writeHead(200, { connection: 'close, x-request-id', 'x-request-id': 'hidden' });
    response.end('ok');
  }, { credential: async () => 'PRIVATE_FIXTURE_KEY' });
  const result = await provider.forward({ path: '/v1/responses', body: body(), headers: {
    Authorization: 'Bearer CALLER_SECRET', Cookie: 'CALLER_COOKIE', 'X-Api-Key': 'CALLER_KEY',
    'X-Forwarded-For': 'CALLER_IP', Accept: 'application/json', Connection: 'OpenAI-Beta',
    'OpenAI-Beta': 'dropped', Host: 'attacker.test', 'Content-Length': '999',
  } });
  assert.equal(result.headers['x-request-id'], undefined);
  assert.equal((await collect(result.body)).toString(), 'ok');
});

test('file and environment credential references stay private and refresh for each call', async (context) => {
  const directory = await mkdtemp(join(tmpdir(), 'provider-test-'));
  context.after(() => rm(directory, { recursive: true, force: true }));
  const file = join(directory, 'authorization');
  await writeFile(file, 'Bearer FIRST_FIXTURE_KEY\n', { mode: 0o600 });
  const received = [];
  const { provider, upstream } = await fixture(context, (request, response) => {
    received.push(request.headers.authorization); request.resume(); response.end('ok');
  }, { authorizationFile: file });
  await collect((await provider.forward({ path: '/v1/responses', body: body() })).body);
  await writeFile(file, 'SECOND_FIXTURE_KEY\n', { mode: 0o600 });
  await collect((await provider.forward({ path: '/v1/responses', body: body() })).body);
  const previous = process.env.PROVIDER_TEST_AUTH;
  process.env.PROVIDER_TEST_AUTH = 'Bearer ENV_FIXTURE_KEY';
  context.after(() => {
    if (previous === undefined) delete process.env.PROVIDER_TEST_AUTH;
    else process.env.PROVIDER_TEST_AUTH = previous;
  });
  const fromEnvironment = createProvider({ upstream, authorizationEnv: 'PROVIDER_TEST_AUTH' });
  context.after(() => fromEnvironment.close());
  await collect((await fromEnvironment.forward({ path: '/v1/responses', body: body() })).body);
  assert.deepEqual(received, ['Bearer FIRST_FIXTURE_KEY', 'Bearer SECOND_FIXTURE_KEY', 'Bearer ENV_FIXTURE_KEY']);
});

test('credential errors and cancellation reasons never expose secrets or internal causes', async (context) => {
  const { provider } = await fixture(context, () => {}, { credential: async () => {
    throw new Error('FAKE_SECRET /private/path');
  } });
  await assert.rejects(provider.forward({ path: '/v1/responses', body: body() }), (error) => {
    assert.ok(error instanceof ProviderError);
    assert.equal(error.code, 'credentials_unavailable');
    assert.ok(!inspect(error).includes('FAKE_SECRET'));
    assert.equal(error.cause, undefined);
    return true;
  });
  const controller = new AbortController(); controller.abort('FAKE_SECRET');
  await assert.rejects(provider.forward({ path: '/v1/responses', body: body(), signal: controller.signal }),
    (error) => error.code === 'request_cancelled' && !inspect(error).includes('FAKE_SECRET'));
});

test('abort cancels credential lookup even when the resolver never settles', async (context) => {
  const started = deferred();
  const controller = new AbortController();
  const { provider } = await fixture(context, () => {}, { credential: ({ signal }) => {
    assert.ok(signal); started.resolve(); return new Promise(() => {});
  } });
  const pending = provider.forward({ path: '/v1/responses', body: body(), signal: controller.signal });
  const rejected = assert.rejects(pending, { code: 'request_cancelled' });
  await started.promise; controller.abort(); await rejected;
});

for (const mode of ['abort', 'return', 'close']) {
  test(`${mode} disposes an unconsumed response socket`, async (context) => {
    const closed = deferred();
    const { provider } = await fixture(context, (request, response) => {
      request.resume(); response.once('close', closed.resolve); response.write('first');
    });
    const controller = new AbortController();
    const result = await provider.forward({ path: '/v1/responses', body: body(), signal: controller.signal });
    if (mode === 'abort') controller.abort();
    if (mode === 'return') await result.body[Symbol.asyncIterator]().return();
    if (mode === 'close') { await provider.close(); await provider.close(); }
    await closed.promise;
    if (mode !== 'return') await assert.rejects(collect(result.body),
      { code: mode === 'close' ? 'provider_closed' : 'request_cancelled' });
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  });
}

test('abort while waiting for response headers releases the socket', async (context) => {
  const arrived = deferred(); const closed = deferred(); const controller = new AbortController();
  const { provider } = await fixture(context, (request, response) => {
    request.resume(); response.once('close', closed.resolve); arrived.resolve();
  });
  const pending = provider.forward({ path: '/v1/responses', body: body(), signal: controller.signal });
  const rejected = assert.rejects(pending, { code: 'request_cancelled' });
  await arrived.promise; controller.abort(); await rejected; await closed.promise;
});

for (const status of [200, 401, 503]) {
  test(`readiness ${status} destroys the response without draining its body`, async (context) => {
    const closed = deferred();
    const { provider } = await fixture(context, (request, response) => {
      assert.equal(request.method, 'GET'); assert.equal(request.url, '/v1/models');
      assert.equal(request.headers.authorization, 'Bearer READY_FIXTURE_KEY');
      response.once('close', closed.resolve); response.writeHead(status);
      response.write(Buffer.alloc(2_000_000, 65));
    }, { authorization: 'READY_FIXTURE_KEY' });
    assert.deepEqual(await provider.ready(), status === 200 ? true
      : { ready: false, reason: 'upstream_unavailable' });
    await closed.promise;
  });
}

test('readiness abort disposes a pending probe and releases its listeners', async (context) => {
  const arrived = deferred(); const closed = deferred(); const controller = new AbortController();
  const { provider } = await fixture(context, (request, response) => {
    request.resume(); response.once('close', closed.resolve); arrived.resolve();
  });
  const pending = provider.ready({ signal: controller.signal });
  const rejected = assert.rejects(pending, { code: 'request_cancelled' });
  await arrived.promise; controller.abort(); await rejected; await closed.promise;
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});

test('readiness reports stable errors and a closed provider refuses new calls', async (context) => {
  const { provider } = await fixture(context, () => {}, { credential: async () => { throw new Error('secret'); } });
  assert.deepEqual(await provider.ready(), { ready: false, reason: 'credentials_unavailable' });
  await provider.close();
  await assert.rejects(provider.ready(), { code: 'provider_closed', status: 503 });
  await assert.rejects(provider.forward({ path: '/v1/responses', body: body() }), { code: 'provider_closed' });
});

test('invalid routes, endpoint credentials and unsupported timer ranges fail closed', async () => {
  for (const upstream of ['invalid', 'file:///tmp/file', 'http://secret@example.test',
    'http://127.0.0.1/?token=secret', 'http://127.0.0.1/#secret']) {
    assert.throws(() => createProvider({ upstream }), ConfigurationError);
  }
  for (const value of [0, -1, 1.5, Infinity, 2_147_483_648, '20']) {
    assert.throws(() => createProvider({ upstream: 'http://127.0.0.1', idleTimeoutMs: value }), ConfigurationError);
    assert.throws(() => createProvider({ upstream: 'http://127.0.0.1', headersTimeoutMs: value }), ConfigurationError);
  }
  const provider = createProvider({ upstream: 'http://127.0.0.1:1' });
  for (const path of ['https://attacker.test', '//attacker.test', '/v1/responses?x=1', '/v1/../responses']) {
    await assert.rejects(provider.forward({ path, body: body() }), { code: 'not_found' });
  }
  await assert.rejects(provider.forward({ path: '/v1/responses', body: Buffer.from('{}') }),
    { code: 'invalid_request_body' });
  await provider.close();
});

test('pre-header network failure is typed and is attempted once', async (context) => {
  let calls = 0;
  const { provider } = await fixture(context, (request) => { calls += 1; request.socket.destroy(); });
  await assert.rejects(provider.forward({ path: '/v1/responses', body: body() }),
    { code: 'upstream_error', status: 502 });
  assert.equal(calls, 1);
});

test('truncated upstream errors retain their status and never reveal the partial body', async (context) => {
  const { provider } = await fixture(context, (request, response) => {
    request.resume(); response.writeHead(401, { 'content-length': 1000 });
    response.write('FAKE_TRUNCATED_SECRET'); setTimeout(() => response.destroy(), 10);
  });
  const result = await provider.forward({ path: '/v1/responses', body: body() });
  assert.equal(result.status, 401);
  assert.equal(JSON.parse(await collect(result.body)).error.message, 'Upstream request failed');
});

test('invalid chunks and failing upload iterators produce safe typed failures', async (context) => {
  const { provider } = await fixture(context, (request) => request.resume());
  for (const source of [Readable.from(['not a Buffer']), (async function* () {
    throw new Error('FAKE_BODY_SECRET');
  })()]) {
    await assert.rejects(provider.forward({ path: '/v1/responses', body: source }), (error) => {
      assert.ok(error instanceof ProviderError); assert.equal(error.status, 400);
      assert.ok(!inspect(error).includes('FAKE_BODY_SECRET')); return true;
    });
  }
});

test('early upstream response stops a pending upload and returns its source', async (context) => {
  const returned = deferred();
  let chunks = 0;
  const source = {
    next() { chunks += 1; return chunks === 1 ? Promise.resolve({ value: Buffer.from('first') }) : new Promise(() => {}); },
    return() { returned.resolve(); return Promise.resolve({ done: true }); },
    [Symbol.asyncIterator]() { return this; },
  };
  const { provider } = await fixture(context, (request, response) => { response.writeHead(413); response.end('secret'); });
  const result = await provider.forward({ path: '/v1/responses', body: source });
  await returned.promise; assert.equal(result.status, 413);
  assert.equal(JSON.parse(await collect(result.body)).error.code, 'upstream_http_error');
});

test('active uploads outlive header timeouts', async (context) => {
  const { provider } = await fixture(context, async (request, response) => {
    await collect(request); response.end('ok');
  }, { headersTimeoutMs: 50, inferenceTimeoutMs: 50, idleTimeoutMs: 80 });
  async function* source() {
    for (let index = 0; index < 8; index += 1) { yield Buffer.from('chunk'); await delay(20); }
  }
  assert.equal((await collect((await provider.forward({ path: '/v1/responses', body: source() })).body)).toString(), 'ok');
});

test('a stalled upload times out and releases a noncooperative iterator', async (context) => {
  const returned = deferred();
  const { provider } = await fixture(context, (request) => request.resume(), { idleTimeoutMs: 40 });
  const source = { next: () => new Promise(() => {}),
    return() { returned.resolve(); return Promise.reject(new Error('ignored cleanup error')); },
    [Symbol.asyncIterator]() { return this; },
  };
  await assert.rejects(provider.forward({ path: '/v1/responses', body: source }), { code: 'idle_timeout' });
  await returned.promise;
});

test('non-HTTP status codes are rejected instead of forwarded', async (context) => {
  const server = net.createServer((socket) => {
    socket.once('data', () => socket.end('HTTP/1.1 600 Invalid\r\nContent-Length: 0\r\n\r\n'));
  });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  const provider = createProvider({ upstream: `http://127.0.0.1:${server.address().port}` });
  context.after(async () => { await provider.close(); await new Promise((done) => server.close(done)); });
  await assert.rejects(provider.forward({ path: '/v1/responses', body: body() }), { code: 'upstream_error' });
});

test('a credential file larger than the private read limit never reaches upstream', async (context) => {
  const directory = await mkdtemp(join(tmpdir(), 'provider-limit-'));
  context.after(() => rm(directory, { recursive: true, force: true }));
  const file = join(directory, 'authorization'); await writeFile(file, 'x'.repeat(20_000), { mode: 0o600 });
  let requests = 0;
  const { provider } = await fixture(context, () => { requests += 1; }, { authorizationFile: file });
  await assert.rejects(provider.forward({ path: '/v1/responses', body: body() }), { code: 'credentials_unavailable' });
  assert.equal(requests, 0);
});

test('upstream disconnect releases an upload blocked on drain', async (context) => {
  const returned = deferred();
  const { provider } = await fixture(context, (request) => {
    request.pause(); setTimeout(() => request.destroy(), 50);
  });
  async function* source() {
    try { while (true) yield Buffer.alloc(65_536); } finally { returned.resolve(); }
  }
  await assert.rejects(provider.forward({ path: '/v1/responses', body: source() }), { code: 'upstream_error' });
  await returned.promise;
});

test('return while a response read is pending cancels immediately', async (context) => {
  const closed = deferred();
  const { provider } = await fixture(context, (request, response) => {
    request.resume(); response.once('close', closed.resolve); response.flushHeaders();
  });
  const result = await provider.forward({ path: '/v1/responses', body: body() });
  const pending = result.body.next();
  const rejected = assert.rejects(pending, { code: 'request_cancelled' });
  await result.body.return(); await rejected; await closed.promise;
});

test('header injection through a private credential fails without reflecting its value', async (context) => {
  const { provider } = await fixture(context, () => {}, { authorization: 'FAKE_SECRET\r\nCookie: bad' });
  await assert.rejects(provider.forward({ path: '/v1/responses', body: body() }), (error) => {
    assert.equal(error.code, 'credentials_unavailable');
    assert.ok(!inspect(error).includes('FAKE_SECRET')); return true;
  });
});

for (const operation of ['forward', 'ready']) {
  test(`${operation}: close shares its promise when credential cancellation reenters shutdown`, async (context) => {
    const started = deferred();
    let reentrant;
    let aborts = 0;
    let lookups = 0;
    const provider = createProvider({ upstream: 'http://127.0.0.1:1', credential: ({ signal }) => {
      lookups += 1;
      signal.addEventListener('abort', () => {
        aborts += 1;
        reentrant = provider.close();
      }, { once: true });
      started.resolve();
      return new Promise(() => {});
    } });
    context.after(() => provider.close());
    const pending = operation === 'forward'
      ? provider.forward({ path: '/v1/responses', body: body() }) : provider.ready();
    const rejected = assert.rejects(pending, { code: 'provider_closed' });
    await started.promise;
    const closing = provider.close();
    assert.equal(provider.close(), closing);
    const refused = Promise.all([
      assert.rejects(provider.forward({ path: '/v1/responses', body: body() }), { code: 'provider_closed' }),
      assert.rejects(provider.ready(), { code: 'provider_closed' }),
    ]);
    await closing;
    await rejected;
    await refused;
    assert.equal(reentrant, closing);
    assert.equal(provider.close(), closing);
    assert.equal(aborts, 1);
    assert.equal(lookups, 1);
  });
}
