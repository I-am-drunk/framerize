import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer } from 'node:http';
import { createServer as createSecureServer } from 'node:https';
import { once, getEventListeners } from 'node:events';
import { setImmediate } from 'node:timers/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { createFramerAuth, FramerAuthError } from '../dist/framer-auth.js';
import { createProvider } from '../dist/provider.js';
import { createServer as createFramerServer } from '../dist/server.js';
import { key, cert } from './helpers/auth-tls.js';

const cookie = 'session=synthetic-private-session';
const token = 'synthetic-private-token';

test('TLS verification cannot be disabled by the process environment', async context => {
  let requests = 0;
  const remote = createSecureServer({ key, cert }, (request, response) => {
    requests += 1;
    assert.equal(request.headers.cookie, undefined);
    response.end('untrusted-control');
  });
  remote.listen(0, '127.0.0.1'); await once(remote, 'listening');
  context.after(async () => {
    remote.closeAllConnections(); await new Promise(resolve => remote.close(resolve));
  });
  const origin = `https://127.0.0.1:${remote.address().port}`;
  await promisify(execFile)(process.execPath, ['--input-type=module', '-e', tlsScript(origin)], {
    timeout: 5000, maxBuffer: 65536,
    env: { SYSTEMROOT: process.env.SYSTEMROOT, NODE_TLS_REJECT_UNAUTHORIZED: '0', NODE_DISABLE_COMPILE_CACHE: '1' },
  });
  assert.equal(requests, 1);
});

test('startup environment proxies never receive the session cookie', async context => {
  const remote = await fixture(context, (request, response) => response.end(JSON.stringify(success())));
  let proxyCalls = 0;
  const proxy = createServer((request, response) => {
    proxyCalls += 1;
    assert.equal(request.headers.cookie, undefined);
    response.end('proxy-control');
  });
  proxy.listen(0, '127.0.0.1'); await once(proxy, 'listening');
  context.after(async () => {
    proxy.closeAllConnections(); await new Promise(resolve => proxy.close(resolve));
  });
  const proxyUrl = `http://127.0.0.1:${proxy.address().port}`;
  const moduleUrl = new URL('../dist/framer-auth.js', import.meta.url).href;
  const script = proxyScript(moduleUrl, remote.origin);
  await promisify(execFile)(process.execPath, ['--input-type=module', '-e', script], {
    timeout: 5000, maxBuffer: 65536,
    env: { SYSTEMROOT: process.env.SYSTEMROOT, NODE_USE_ENV_PROXY: '1',
      HTTP_PROXY: proxyUrl, HTTPS_PROXY: proxyUrl, NO_PROXY: '', NODE_DISABLE_COMPILE_CACHE: '1' },
  });
  assert.equal(proxyCalls, Number(process.versions.node.split('.')[0]) >= 24 ? 1 : 0);
  assert.equal(remote.calls(), 1);
});

for (const input of [null, {}, { sessionCookie: '' }, { sessionCookie: 'x\r\ny' },
  { sessionCookie: cookie, origin: 'http://example.test' },
  { sessionCookie: cookie, origin: 'https://example.test/path' },
  { sessionCookie: cookie, origin: 'https://user:secret@example.test' },
  { sessionCookie: cookie, timeoutMs: 0 }, { sessionCookie: cookie, timeoutMs: Infinity },
  { get origin() { throw new Error('synthetic-private-options'); } },
]) {
  test('invalid authentication options have fixed diagnostics', () => {
    assert.throws(() => createFramerAuth(input), { message: 'Invalid Framer authentication configuration' });
  });
}

const success = () => ({ accessToken: token, expiresAt: new Date(Date.now() + 600_000).toISOString() });

for (const [status, code] of [[401, 'session_unavailable'], [403, 'auth_refused'],
  [302, 'auth_unavailable'], [429, 'auth_unavailable'], [500, 'auth_unavailable']]) {
  test(`HTTP ${status} has a fixed error and never retries or follows redirects`, async context => {
    const remote = await fixture(context, (request, response) => {
      response.writeHead(status, { location: '/must-not-follow' });
      response.end('synthetic-private-error-body');
    });
    const auth = createFramerAuth({ sessionCookie: cookie, origin: remote.origin });
    context.after(() => auth.close());
    await assert.rejects(auth.accessToken(), failure(code));
    assert.equal(remote.calls(), 1);
  });
}

test('pre-aborted requests and invalid contexts make no network request', async context => {
  const remote = await fixture(context, (request, response) => response.end(JSON.stringify(success())));
  const auth = createFramerAuth({ sessionCookie: cookie, origin: remote.origin });
  context.after(() => auth.close());
  await assert.rejects(auth.accessToken({ signal: AbortSignal.abort() }), failure('auth_aborted'));
  await assert.rejects(auth.accessToken({ get signal() { throw new Error(cookie); } }), failure('auth_unavailable'));
  await assert.rejects(auth.accessToken({ signal: {} }), failure('auth_unavailable'));
  const signal = new Proxy(new AbortController().signal, {
    get() { throw new Error('synthetic-private-signal'); },
  });
  await assert.rejects(auth.accessToken({ signal }), failure('auth_unavailable'));
  assert.equal(remote.calls(), 0);
});
const failure = code => error => {
  assert.ok(error instanceof FramerAuthError);
  assert.equal(error.code, code);
  assert.equal(error.message, 'Framer authentication unavailable');
  assert.doesNotMatch(error.stack + JSON.stringify(error), /synthetic-private/);
  return true;
};

function deferred() {
  let resolve;
  const promise = new Promise(accept => { resolve = accept; });
  return { promise, resolve };
}

function tlsScript(origin) {
  const moduleUrl = new URL('../dist/framer-auth.js', import.meta.url).href;
  const backendUrl = new URL('../dist/framer-backend.js', import.meta.url).href;
  return `import assert from 'node:assert/strict';
import https from 'node:https';
import { createFramerAuth } from ${JSON.stringify(moduleUrl)};
const origin = ${JSON.stringify(origin)};
const body = await new Promise((resolve, reject) => {
  https.get(origin, response => {
    let body = ''; response.on('data', chunk => body += chunk);
    response.on('end', () => resolve(body)); response.on('error', reject);
  }).on('error', reject);
});
assert.equal(body, 'untrusted-control');
https.globalAgent.destroy();
const auth = createFramerAuth({ origin, sessionCookie: ${JSON.stringify(cookie)} });
try { await assert.rejects(auth.accessToken(), { code: 'auth_unavailable' }); }
finally { await auth.close(); }
const { createFramerBackend } = await import(${JSON.stringify(backendUrl)});
const backend = createFramerBackend({ origin, projectId: 'synthetic',
  auth: { async accessToken() { return 'synthetic'; }, async close() {} },
  castle: { async mint() { return { token: 'synthetic' }; }, async close() {} } });
try { await assert.rejects(backend.forward({ path: '/v1/responses', headers: {},
  signal: new AbortController().signal,
  body: (async function* () { yield Buffer.from('{"model":"synthetic","input":"test"}'); })(),
}), { code: 'upstream_error' }); }
finally { await backend.close(); }
await new Promise(resolve => setImmediate(resolve));
assert.deepEqual(process.getActiveResourcesInfo().filter(name => /TCP|Timeout/.test(name)), []);
`;
}

test('the real provider consumes cached auth without forwarding the session cookie', async context => {
  let authCalls = 0;
  let forwarded = 0;
  let available = true;
  const bytes = 'data: {"type":"response.completed"}\n\n';
  const remote = createServer((request, response) => {
    request.resume();
    if (request.url === '/auth/ai/access-token') {
      authCalls += 1;
      assert.equal(request.headers.cookie, cookie);
      assert.equal(request.headers.authorization, undefined);
      response.writeHead(available ? 200 : 401, { 'content-type': 'application/json' });
      response.end(JSON.stringify(success()));
    } else {
      forwarded += 1;
      assert.equal(request.headers.cookie, undefined);
      assert.equal(request.headers.authorization, `Bearer ${token}`);
      response.setHeader('content-type', 'text/event-stream'); response.end(bytes);
    }
  });
  remote.listen(0, '127.0.0.1'); await once(remote, 'listening');
  context.after(async () => {
    remote.closeAllConnections(); await new Promise(resolve => remote.close(resolve));
  });
  const origin = `http://127.0.0.1:${remote.address().port}`;
  const auth = createFramerAuth({ sessionCookie: cookie, origin });
  const transport = createProvider({ upstream: origin, credential: context => auth.accessToken(context) });
  const provider = { ...transport, async close() { await Promise.all([transport.close(), auth.close()]); } };
  const app = await createFramerServer({ host: '127.0.0.1', port: 0, serverToken: 'synthetic-local' }, { provider });
  context.after(() => app.close());
  const local = `http://127.0.0.1:${(await app.listen()).port}`;
  const headers = { authorization: 'Bearer synthetic-local', 'content-type': 'application/json' };
  const refused = await fetch(`${local}/ready`);
  assert.equal(refused.status, 401); await refused.arrayBuffer();
  assert.equal(authCalls, 0);
  const ready = await fetch(`${local}/ready`, { headers });
  assert.equal(ready.status, 200); await ready.arrayBuffer();
  const result = await fetch(`${local}/v1/responses`, { method: 'POST', headers, body: '{}' });
  assert.equal(result.status, 200); assert.equal(await result.text(), bytes);
  assert.equal(authCalls, 1);
  available = false;
  auth.invalidate(token);
  const unavailable = await fetch(`${local}/ready`, { headers });
  assert.equal(unavailable.status, 503);
  assert.equal((await unavailable.json()).reason, 'credentials_unavailable');
  assert.equal(authCalls, 2);
  assert.equal(forwarded, 2);
  await app.close();
  await assert.rejects(auth.accessToken(), failure('auth_closed'));
});

function proxyScript(moduleUrl, origin) {
  return `import assert from 'node:assert/strict';
import http from 'node:http';
import { createFramerAuth } from ${JSON.stringify(moduleUrl)};
const origin = ${JSON.stringify(origin)};
if (Number(process.versions.node.split('.')[0]) >= 24) {
  const body = await new Promise((resolve, reject) => {
    http.get(origin, response => {
      let body = ''; response.on('data', chunk => body += chunk);
      response.on('end', () => resolve(body)); response.on('error', reject);
    }).on('error', reject);
  });
  assert.equal(body, 'proxy-control');
  http.globalAgent.destroy();
}
const auth = createFramerAuth({ sessionCookie: ${JSON.stringify(cookie)}, origin });
try { assert.equal(await auth.accessToken(), ${JSON.stringify(token)}); }
finally { await auth.close(); }
await new Promise(resolve => setImmediate(resolve));
assert.deepEqual(process.getActiveResourcesInfo().filter(name => /TCP|Timeout/.test(name)), []);
`;
}

for (const stage of ['headers', 'body']) {
  test(`the total deadline closes a stalled ${stage} exchange`, async context => {
    const entered = deferred();
    const remote = await fixture(context, (request, response) => {
      if (stage === 'body') response.write('{');
      entered.resolve(response);
    });
    const auth = createFramerAuth({ sessionCookie: cookie, origin: remote.origin, timeoutMs: 100 });
    context.after(() => auth.close());
    const refused = assert.rejects(auth.accessToken(), failure('auth_timeout'));
    const response = await entered.promise;
    const ended = once(response, 'close');
    await refused; await ended; await setImmediate();
    assert.equal(remote.sockets.size, 0);
  });
}

for (const headers of [{ 'content-type': 'text/html' }, { 'content-encoding': 'gzip' },
  { 'x-large': 'x'.repeat(17_000) }, { 'x-repeat': Array(3400).fill('a') }]) {
  test(`refuses invalid response headers ${Object.keys(headers)[0]}`, async context => {
    const remote = await fixture(context, (request, response) => {
      response.writeHead(200, headers); response.end(JSON.stringify(success()));
    });
    const auth = createFramerAuth({ sessionCookie: cookie, origin: remote.origin });
    context.after(() => auth.close());
    await assert.rejects(auth.accessToken(), failure('auth_unavailable'));
  });
}

for (const [label, body] of [
  ['malformed JSON', () => '{'], ['non-object JSON', () => 'null'],
  ['empty bearer', () => JSON.stringify({ ...success(), accessToken: '' })],
  ['invalid bearer', () => JSON.stringify({ ...success(), accessToken: 'bad\r\nvalue' })],
  ['non-bearer syntax', () => JSON.stringify({ ...success(), accessToken: '"quoted"' })],
  ['expired token', () => JSON.stringify({ ...success(), expiresAt: '2020-01-01T00:00:00Z' })],
  ['invalid expiry', () => JSON.stringify({ ...success(), expiresAt: 'not-a-date' })],
  ['impossible date', () => JSON.stringify({ ...success(), expiresAt: '2999-02-30T00:00:00Z' })],
  ['invalid expiry hour', () => JSON.stringify({ ...success(), expiresAt: '2999-03-01T24:00:00Z' })],
  ['oversized body', () => ' '.repeat(65_537)],
  ['invalid UTF-8', () => Buffer.from([0xff, 0xfe])],
]) {
  test(`rejects ${label} without caching or leaking the body`, async context => {
    const remote = await fixture(context, (request, response, calls) => {
      response.end(calls === 1 ? body() : JSON.stringify(success()));
    });
    const auth = createFramerAuth({ sessionCookie: cookie, origin: remote.origin });
    context.after(() => auth.close());
    await assert.rejects(auth.accessToken(), failure('auth_unavailable'));
    assert.equal(await auth.accessToken(), token);
    assert.equal(remote.calls(), 2);
  });
}

test('instances do not share cache and refresh with the 30-second safety margin', async context => {
  context.mock.timers.enable({ apis: ['Date'], now: Date.parse('2026-11-03T12:00:00Z') });
  const remote = await fixture(context, (request, response, calls) => {
    response.end(JSON.stringify({ accessToken: `${token}-${calls}`,
      expiresAt: new Date(Date.now() + 86_400_000).toISOString() }));
  });
  const first = createFramerAuth({ sessionCookie: cookie, origin: remote.origin });
  const second = createFramerAuth({ sessionCookie: cookie, origin: remote.origin });
  context.after(() => Promise.all([first.close(), second.close()]));
  assert.equal(await first.accessToken(), `${token}-1`);
  assert.equal(await second.accessToken(), `${token}-2`);
  context.mock.timers.tick(569_999);
  assert.equal(await first.accessToken(), `${token}-1`);
  context.mock.timers.tick(1);
  assert.equal(await first.accessToken(), `${token}-3`);
  assert.equal(remote.calls(), 3);
});

test('cancelling one waiter preserves the shared request for another waiter', async context => {
  const entered = deferred();
  const remote = await fixture(context, (request, response) => entered.resolve(response));
  const auth = createFramerAuth({ sessionCookie: cookie, origin: remote.origin });
  context.after(() => auth.close());
  const controller = new AbortController();
  const refused = assert.rejects(auth.accessToken({ signal: controller.signal }), failure('auth_aborted'));
  const survivor = auth.accessToken();
  const response = await entered.promise;
  controller.abort(new Error('synthetic-private-abort'));
  await refused;
  assert.equal(remote.calls(), 1);
  assert.equal(response.destroyed, false);
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  response.end(JSON.stringify(success()));
  assert.equal(await survivor, token);
});

async function fixture(context, handler) {
  let calls = 0;
  const sockets = new Set();
  const server = createServer((request, response) => {
    calls += 1;
    assert.equal(request.method, 'GET');
    assert.equal(request.url, '/auth/ai/access-token');
    assert.equal(request.headers.cookie, cookie);
    assert.equal(request.headers.authorization, undefined);
    request.resume();
    response.setHeader('content-type', 'application/json');
    handler(request, response, calls);
  });
  server.on('connection', socket => {
    sockets.add(socket); socket.on('close', () => sockets.delete(socket));
  });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  context.after(async () => {
    server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
  });
  return { origin: `http://127.0.0.1:${server.address().port}`, sockets, calls: () => calls };
}

test('the final cancelled waiter closes the request before the next acquisition', async context => {
  const entered = deferred();
  const remote = await fixture(context, (request, response, calls) => {
    if (calls === 1) entered.resolve(response);
    else response.end(JSON.stringify(success()));
  });
  const auth = createFramerAuth({ sessionCookie: cookie, origin: remote.origin });
  context.after(() => auth.close());
  const controller = new AbortController();
  const refused = assert.rejects(auth.accessToken({ signal: controller.signal }), failure('auth_aborted'));
  const response = await entered.promise;
  const ended = once(response, 'close');
  controller.abort(new Error('synthetic-private-abort'));
  const next = auth.accessToken();
  await refused; await ended;
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  assert.equal(await next, token);
  assert.equal(remote.calls(), 2);
});

test('close cancels every waiter and is idempotent', async context => {
  const entered = deferred();
  const remote = await fixture(context, (request, response) => entered.resolve(response));
  const auth = createFramerAuth({ sessionCookie: cookie, origin: remote.origin });
  context.after(() => auth.close());
  const controllers = [new AbortController(), new AbortController()];
  const rejected = controllers.map(controller => assert.rejects(
    auth.accessToken({ signal: controller.signal }), failure('auth_closed')));
  const response = await entered.promise;
  const ended = once(response, 'close');
  const closing = auth.close();
  assert.equal(closing, auth.close());
  await Promise.all([...rejected, closing, ended]);
  for (const controller of controllers) assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  assert.equal(remote.calls(), 1);
  await assert.rejects(auth.accessToken(), failure('auth_closed'));
});

test('one session shares token acquisition, caches, and invalidates only the rejected token', async context => {
  const remote = await fixture(context, (request, response, calls) => {
    response.end(JSON.stringify({ ...success(), accessToken: `${token}-${calls}` }));
  });
  const auth = createFramerAuth({ sessionCookie: cookie, origin: remote.origin });
  context.after(() => auth.close());
  const controllers = Array.from({ length: 12 }, () => new AbortController());
  const results = await Promise.all(controllers.map(controller => auth.accessToken({ signal: controller.signal })));
  for (const controller of controllers) assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  assert.deepEqual(results, Array(12).fill(`${token}-1`));
  assert.equal(await auth.accessToken(), `${token}-1`);
  assert.equal(remote.calls(), 1);
  auth.invalidate(`${token}-1`);
  assert.equal(await auth.accessToken(), `${token}-2`);
  auth.invalidate(`${token}-1`);
  assert.equal(await auth.accessToken(), `${token}-2`);
  assert.equal(remote.calls(), 2);
  assert.equal(JSON.stringify(auth), '{}');
  const closing = auth.close();
  assert.equal(auth.close(), closing);
  await closing;
  await assert.rejects(auth.accessToken(), failure('auth_closed'));
});
