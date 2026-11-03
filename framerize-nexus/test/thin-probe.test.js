import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import net from 'node:net';
import { getEventListeners } from 'node:events';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { gzipSync } from 'node:zlib';
import { probeReadiness } from '../src/thin/probe.js';

const TOKEN = 'synthetic-probe-token';
const CANARY = 'SYNTHETIC_PROBE_SECRET_93f1';
const READY = { apiVersion: 1, status: 'ready',
  capabilities: { responses: true, chatCompletions: true, streaming: true } };
const bytes = (value) => Buffer.from(JSON.stringify(value));
const execute = promisify(execFile);

async function peer(context, handler, raw = false) {
  const sockets = new Set(), intervals = new Set(), requests = [];
  const server = raw ? net.createServer(handler) : http.createServer((request, response) => {
    requests.push({ method: request.method, url: request.url, headers: request.headers });
    handler(request, response);
  });
  server.on('connection', (socket) => {
    sockets.add(socket);
    socket.on('error', () => {});
    if (raw) socket.resume();
    socket.once('close', () => sockets.delete(socket));
  });
  context.after(async () => {
    for (const timer of intervals) clearInterval(timer);
    for (const socket of sockets) socket.destroy();
    await new Promise((resolve) => server.close(resolve));
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  return { server, sockets, intervals, requests, upstream: `http://127.0.0.1:${server.address().port}`,
    repeat(socket, callback, delay = 20) {
      const timer = setInterval(callback, delay);
      intervals.add(timer);
      socket.once('close', () => { clearInterval(timer); intervals.delete(timer); });
    } };
}

async function closed(fixture) {
  if (fixture.sockets.size) await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('fixture connection leaked')), 2000);
    const check = () => {
      if (!fixture.sockets.size) { clearTimeout(timer); resolve(); }
    };
    for (const socket of fixture.sockets) socket.once('close', check);
  });
  assert.equal(fixture.sockets.size, 0);
  assert.equal(fixture.intervals.size, 0);
}

async function tracked(context, options, inspect = () => {}) {
  const timers = new Set(), delays = [];
  const originalSet = globalThis.setTimeout, originalClear = globalThis.clearTimeout;
  const signal = options.signal ?? new AbortController().signal;
  const listeners = getEventListeners(signal, 'abort').length;
  const setMock = context.mock.method(globalThis, 'setTimeout', (callback, delay, ...args) => {
    const timer = originalSet((...values) => { timers.delete(timer); callback(...values); }, delay, ...args);
    timers.add(timer);
    delays.push(delay);
    return timer;
  });
  const clearMock = context.mock.method(globalThis, 'clearTimeout', (timer) => {
    timers.delete(timer);
    return originalClear(timer);
  });
  try { return await probeReadiness({ ...options, signal }); }
  finally {
    setMock.mock.restore();
    clearMock.mock.restore();
    assert.equal(timers.size, 0, 'all probe timers must be cleared');
    assert.equal(getEventListeners(signal, 'abort').length, listeners);
    inspect(delays);
  }
}

function safeError(code) {
  return (error) => {
    assert.equal(error.code, `ERR_READINESS_${code}`);
    assert.equal(error.cause, undefined);
    assert.equal(error.message, {
      OPTIONS: 'invalid readiness options', ABORTED: 'readiness request aborted',
      HEADERS_TIMEOUT: 'readiness headers timeout', IDLE_TIMEOUT: 'readiness body idle timeout',
      TIMEOUT: 'readiness exchange timeout', SIZE: 'readiness response exceeds byte limit',
      RESPONSE: 'invalid readiness response', NETWORK: 'readiness request failed',
    }[code]);
    for (const text of [error.message, error.stack, String(error), JSON.stringify(error)]) {
      assert.ok(!text.includes(CANARY));
      assert.ok(!text.includes(TOKEN));
    }
    return true;
  };
}

test('invalid options reject without coercion or secret-bearing diagnostics', async () => {
  let coercions = 0;
  const value = { toString() { coercions += 1; return CANARY; } };
  const valid = { upstream: 'http://127.0.0.1', remoteToken: TOKEN };
  for (const upstream of [undefined, null, 1, value, '', 'ftp://host', `https://${CANARY}@host`,
    `http://host/${CANARY}`, 'http://host/', 'http://HOST', 'http://host:80', 'http://host?']) {
    await assert.rejects(probeReadiness({ ...valid, upstream }), safeError('OPTIONS'));
  }
  for (const remoteToken of [undefined, null, 1, value, '', ' ', 'Bearer token', `${CANARY}\n`, 'bad=middle']) {
    await assert.rejects(probeReadiness({ ...valid, remoteToken }), safeError('OPTIONS'));
  }
  assert.equal(coercions, 0);
});

test('limits and option getters fail safely before opening a request', async () => {
  const valid = { upstream: 'http://127.0.0.1', remoteToken: TOKEN };
  for (const name of ['headersTimeoutMs', 'idleTimeoutMs', 'readinessTimeoutMs', 'maxReadinessBytes']) {
    for (const value of [0, -1, 1.5, '100', Infinity, NaN, null, Number.MAX_SAFE_INTEGER + 1]) {
      await assert.rejects(probeReadiness({ ...valid, [name]: value }), safeError('OPTIONS'));
    }
    if (name !== 'maxReadinessBytes') await assert.rejects(probeReadiness({ ...valid, [name]: 2147483648 }), safeError('OPTIONS'));
  }
  for (const options of [undefined, null, [], { ...valid, signal: {} },
    { ...valid, signal: Object.create(AbortSignal.prototype) },
    { get upstream() { throw new Error(CANARY); } }]) {
    await assert.rejects(probeReadiness(options), safeError('OPTIONS'));
  }
});

test('ready preserves exact bytes, uses only root readiness and configured auth', async (context) => {
  const body = Buffer.concat([Buffer.from(' \n'), bytes(READY), Buffer.from('\t')]);
  const fixture = await peer(context, (request, response) => response.end(body));
  const options = Object.freeze({ upstream: fixture.upstream, remoteToken: 'token_~+/-==' });
  const result = await tracked(context, options, (delays) => assert.deepEqual(delays.slice(0, 3), [20000, 15000, 5000]));
  assert.deepEqual(result.body, body);
  assert.deepEqual(result.readiness, { state: 'ready' });
  assert.ok(Object.isFrozen(result.readiness));
  assert.equal(result.status, 200);
  assert.deepEqual(fixture.requests, [{ method: 'GET', url: '/ready', headers: {
    authorization: `Bearer ${options.remoteToken}`, accept: 'application/json', 'accept-encoding': 'identity',
    host: new URL(fixture.upstream).host, connection: 'close' } }]);
  await closed(fixture);
});

test('completed responses preserve status and classify without translating failures', async (context) => {
  const cases = [
    [503, bytes({ ...READY, status: 'unavailable', reason: 'upstream_unavailable' }), 'unavailable'],
    [200, bytes({ ...READY, apiVersion: 2 }), 'incompatible'],
    [200, bytes({ ...READY, capabilities: { ...READY.capabilities, streaming: false } }), 'incompatible'],
    [200, Buffer.from([0xff, 0xfe]), 'malformed'], [200, Buffer.from('{'), 'malformed'],
    [401, bytes({ error: 'unauthorized' }), 'malformed'], [302, bytes(READY), 'unavailable'],
  ];
  let index = 0;
  const fixture = await peer(context, (request, response) => {
    response.writeHead(cases[index][0], { location: `${fixture.upstream}/must-not-follow` });
    response.end(cases[index][1]);
  });
  for (const [status, body, state] of cases) {
    const result = await tracked(context, { upstream: fixture.upstream, remoteToken: TOKEN });
    assert.equal(result.status, status);
    assert.deepEqual(result.body, body);
    assert.equal(result.readiness.state, state);
    assert.equal(result.headers.location, undefined);
    await closed(fixture);
    index += 1;
  }
  assert.equal(fixture.requests.length, cases.length);
});

test('response allowlist removes Connection nominations and cookies without decoding gzip', async (context) => {
  const body = gzipSync(bytes(READY));
  const fixture = await peer(context, (request, response) => {
    response.writeHead(200, { connection: 'X-ReQUeSt-ID, Content-Type, content-encoding',
      'content-type': 'application/json', 'content-encoding': 'gzip', 'content-length': body.length,
      'set-cookie': CANARY, 'x-request-id': CANARY, 'cache-control': 'no-store', 'retry-after': '1',
      'x-unapproved': CANARY });
    response.end(body);
  });
  const result = await tracked(context, { upstream: fixture.upstream, remoteToken: TOKEN });
  assert.deepEqual(result.body, body);
  assert.deepEqual(result.readiness, { state: 'malformed' });
  assert.ok(Object.isFrozen(result.readiness));
  assert.deepEqual(result.headers, { 'content-length': String(body.length), 'cache-control': 'no-store', 'retry-after': '1' });
  await closed(fixture);
});

test('default 64KiB and explicit byte boundaries accept exact identity bodies', async (context) => {
  let body;
  const fixture = await peer(context, (request, response) => {
    response.writeHead(200, { 'content-encoding': 'IDENTITY', 'x-request-id': 'fixture-id' });
    response.end(body);
  });
  for (const limit of [undefined, 128]) {
    body = Buffer.alloc(limit ?? 65536, ' ');
    bytes(READY).copy(body);
    const result = await tracked(context, { upstream: fixture.upstream, remoteToken: TOKEN, maxReadinessBytes: limit });
    assert.deepEqual(result.body, body);
    assert.equal(result.readiness.state, 'ready');
    assert.equal(result.headers['content-encoding'], 'IDENTITY');
    await closed(fixture);
  }
});

test('oversized advertised bodies reject before waiting for bytes at every status', async (context) => {
  let status = 200;
  const fixture = await peer(context, (request, response) => {
    response.writeHead(status, { 'content-length': '65537' });
    response.flushHeaders();
  });
  for (status of [200, 302, 401, 500]) {
    await assert.rejects(tracked(context, { upstream: fixture.upstream, remoteToken: TOKEN }), safeError('SIZE'));
    await closed(fixture);
  }
});

test('chunked bodies are capped before retention for success and error statuses', async (context) => {
  const concatenated = [];
  const original = Buffer.concat;
  context.mock.method(Buffer, 'concat', (chunks, length) => {
    if (chunks.length && chunks.every((chunk) => chunk.every((value) => value === 0))) concatenated.push(length);
    return original(chunks, length);
  });
  let status = 200;
  const fixture = await peer(context, (request, response) => {
    response.writeHead(status);
    response.write(Buffer.alloc(32));
    response.end(Buffer.alloc(33));
  });
  for (status of [200, 302, 401, 500]) {
    await assert.rejects(tracked(context, { upstream: fixture.upstream, remoteToken: TOKEN,
      maxReadinessBytes: 64 }), safeError('SIZE'));
    await closed(fixture);
  }
  assert.deepEqual(concatenated, [], 'oversized bytes must not be concatenated');
});

test('missing and incomplete headers hit the header bound and close peers', async (context) => {
  for (const partial of [false, true]) {
    const fixture = await peer(context, (socket) => {
      if (partial) socket.write('HTTP/1.1 200 OK\r\nX-Test: incomplete');
    }, true);
    await assert.rejects(tracked(context, { upstream: fixture.upstream, remoteToken: TOKEN,
      headersTimeoutMs: 100, readinessTimeoutMs: 2000 }), safeError('HEADERS_TIMEOUT'));
    await closed(fixture);
  }
});

test('body inactivity has a separate bounded timeout', async (context) => {
  const fixture = await peer(context, (request, response) => {
    response.writeHead(200);
    response.write('{');
  });
  await assert.rejects(tracked(context, { upstream: fixture.upstream, remoteToken: TOKEN,
    idleTimeoutMs: 100, readinessTimeoutMs: 2000 }), safeError('IDLE_TIMEOUT'));
  await closed(fixture);
});

test('an endless incomplete body hits the whole-exchange bound despite activity', async (context) => {
  const fixture = await peer(context, (request, response) => {
    response.writeHead(200);
    response.write('{');
    fixture.repeat(request.socket, () => response.write(' '));
  });
  await assert.rejects(tracked(context, { upstream: fixture.upstream, remoteToken: TOKEN,
    headersTimeoutMs: 1500, idleTimeoutMs: 500, readinessTimeoutMs: 300 }), safeError('TIMEOUT'));
  await closed(fixture);
});

test('an already-aborted signal opens no connection or timers', async (context) => {
  const fixture = await peer(context, () => assert.fail('must not connect'));
  const controller = new AbortController();
  controller.abort(new Error(CANARY));
  await assert.rejects(tracked(context, { upstream: fixture.upstream, remoteToken: TOKEN,
    signal: controller.signal }, (delays) => assert.deepEqual(delays, [])), safeError('ABORTED'));
  assert.equal(fixture.requests.length, 0);
  await closed(fixture);
});

test('aborting during headers, body or trickle releases timers, listeners and sockets', async (context) => {
  for (const phase of ['headers', 'body', 'trickle']) {
    const controller = new AbortController();
    const fixture = await peer(context, (request, response) => {
      if (phase !== 'headers') { response.writeHead(200); response.write('{'); }
      let ticks = 0;
      fixture.repeat(request.socket, () => {
        if (phase === 'trickle') response.write(' ');
        if (++ticks === 3) controller.abort({ message: CANARY, path: `/private/${CANARY}` });
      });
    });
    await assert.rejects(tracked(context, { upstream: fixture.upstream, remoteToken: TOKEN,
      signal: controller.signal }), safeError('ABORTED'));
    await closed(fixture);
  }
});

test('malformed, truncated, upgraded and reset exchanges reject safely', async (context) => {
  const cases = [
    [`NOT HTTP ${CANARY}\r\n`, 'NETWORK'],
    [`HTTP/1.1 200 OK\r\nContent-Length: 999\r\n\r\n${CANARY}`, 'RESPONSE'],
    [`HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nZ\r\n${CANARY}`, 'NETWORK'],
    ['HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: fixture\r\n\r\n', 'RESPONSE'],
    [null, 'NETWORK'],
  ];
  for (const [wire, code] of cases) {
    const fixture = await peer(context, (socket) => socket.once('data', () => {
      if (wire === null) socket.destroy(); else socket.end(wire);
    }), true);
    await assert.rejects(tracked(context, { upstream: fixture.upstream, remoteToken: TOKEN }), safeError(code));
    await closed(fixture);
  }
});

test('native synchronous failures and TLS errors expose only safe errors', async (context) => {
  const requestMock = context.mock.method(http, 'request', () => { throw new Error(CANARY); });
  await assert.rejects(tracked(context, { upstream: 'http://127.0.0.1', remoteToken: TOKEN }), safeError('NETWORK'));
  requestMock.mock.restore();
  const fixture = await peer(context, (socket) => socket.once('data', () => socket.end(CANARY)), true);
  await assert.rejects(tracked(context, { upstream: fixture.upstream.replace('http:', 'https:'),
    remoteToken: TOKEN }), safeError('NETWORK'));
  await closed(fixture);
});

test('repeated and concurrent calls use independent requests without cached verdicts', async (context) => {
  const ports = new Set();
  let calls = 0;
  const fixture = await peer(context, (request, response) => {
    ports.add(request.socket.remotePort);
    response.end(bytes(++calls === 2 ? { ...READY, status: 'unavailable', reason: 'upstream_unavailable' } : READY));
  });
  const options = Object.freeze({ upstream: fixture.upstream, remoteToken: TOKEN });
  assert.equal((await tracked(context, options)).readiness.state, 'ready');
  assert.equal((await tracked(context, options)).readiness.state, 'unavailable');
  const results = await Promise.all(Array.from({ length: 3 }, () => probeReadiness(options)));
  assert.ok(results.every((result) => result.readiness.state === 'ready'));
  assert.equal(calls, 5);
  assert.equal(ports.size, 5);
  await closed(fixture);
});

test('startup environment proxies cannot intercept the owned probe agent', async (context) => {
  const fixture = await peer(context, (request, response) => response.end(bytes(READY)));
  const proxy = await peer(context, (request, response) => response.end('proxy control'));
  const home = mkdtempSync(join(tmpdir(), 'nexus-probe-env-'));
  context.after(() => rmSync(home, { recursive: true, force: true }));
  const environment = { HOME: home, USERPROFILE: home, TMPDIR: home, TEMP: home, TMP: home,
    PATH: process.env.PATH, SystemRoot: process.env.SystemRoot, NODE_USE_ENV_PROXY: '1',
    HTTP_PROXY: proxy.upstream, HTTPS_PROXY: proxy.upstream, ALL_PROXY: proxy.upstream,
    http_proxy: proxy.upstream, https_proxy: proxy.upstream, all_proxy: proxy.upstream,
    NO_PROXY: '', no_proxy: '', FRAMERIZE_NEXUS_UPSTREAM: `https://${CANARY}.invalid`,
    FRAMERIZE_NEXUS_REMOTE_TOKEN_FILE: `/private/${CANARY}`, NODE_OPTIONS: '' };
  const moduleUrl = new URL('../src/thin/probe.js', import.meta.url).href;
  const script = [
    "import http from 'node:http';",
    `import { probeReadiness } from ${JSON.stringify(moduleUrl)};`,
    "if (Number(process.versions.node.split('.')[0]) >= 24) await new Promise((resolve, reject) => {",
    "http.get(process.argv[1], (response) => { response.resume(); response.on('end', resolve); }).on('error', reject); });",
    `const result = await probeReadiness({ upstream: process.argv[1], remoteToken: '${TOKEN}' });`,
    "console.log(JSON.stringify({ status: result.status, state: result.readiness.state }));",
  ].join('\n');
  const child = await execute(process.execPath, ['--disable-warning=UNDICI-EHPA', '--input-type=module', '--eval', script, '--', fixture.upstream],
    { env: environment, timeout: 10000, maxBuffer: 8192 });
  assert.deepEqual(JSON.parse(child.stdout), { status: 200, state: 'ready' });
  assert.equal(child.stderr, '');
  assert.equal(fixture.requests.length, 1);
  assert.equal(proxy.requests.length, Number(process.versions.node.split('.')[0]) >= 24 ? 1 : 0);
  assert.ok(proxy.requests.every((request) => request.headers.authorization === undefined));
  await closed(fixture);
  await closed(proxy);
});

test('oversized response headers reject safely and release resources', async (context) => {
  for (const fields of [`X-Test: ${CANARY}${'a'.repeat(16384)}\r\n`, 'x:a\r\n'.repeat(10000)]) {
    const body = bytes(READY);
    const headers = `HTTP/1.1 200 OK\r\n${fields}Connection: close\r\nContent-Length: ${body.length}\r\n\r\n`;
    assert.ok(Buffer.byteLength(headers) > 16384 && Buffer.byteLength(headers) < 65536);
    const fixture = await peer(context, (socket) => socket.once('data', () => {
      socket.end(Buffer.concat([Buffer.from(headers), body]));
    }), true);
    await assert.rejects(tracked(context, { upstream: fixture.upstream, remoteToken: TOKEN }), safeError('NETWORK'));
    await closed(fixture);
  }
});

test('late Connection nominations survive more than 1000 header pairs', async (context) => {
  const body = bytes(READY);
  for (const count of [0, 2100]) {
    const headers = `HTTP/1.1 200 OK\r\nX-Request-Id: ${CANARY}\r\n${'x:a\r\n'.repeat(count)}`
      + `Connection: X-Request-ID, close\r\nContent-Length: ${body.length}\r\n\r\n`;
    assert.ok(Buffer.byteLength(headers) < 16384);
    const fixture = await peer(context, (socket) => socket.once('data', () => {
      socket.end(Buffer.concat([Buffer.from(headers), body]));
    }), true);
    const result = await tracked(context, { upstream: fixture.upstream, remoteToken: TOKEN });
    assert.equal(result.headers['x-request-id'], undefined);
    assert.deepEqual(result.body, body);
    assert.equal(result.readiness.state, 'ready');
    await closed(fixture);
  }
});
