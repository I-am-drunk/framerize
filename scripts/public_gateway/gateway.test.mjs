import { test } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { once } from 'node:events';
import { createGateway } from './gateway.mjs';

async function setup(t, handler, options = {}) {
  const upstream = http.createServer(handler);
  upstream.listen(0, '127.0.0.1'); await once(upstream, 'listening');
  const gateway = createGateway({ target: `http://127.0.0.1:${upstream.address().port}`, ...options });
  gateway.listen(0, '127.0.0.1'); await once(gateway, 'listening');
  t.after(() => { gateway.closeAllConnections(); gateway.close(); upstream.closeAllConnections(); upstream.close(); });
  return `http://127.0.0.1:${gateway.address().port}`;
}

test('both dialects share the root and preserve explicit route prefixes', async t => {
  const base = await setup(t, (req, res) => res.end(req.url));
  for (const [path, expected] of [
    ['/responses', '/openai/v1/responses'], ['/v1/responses', '/openai/v1/responses'],
    ['/openai/v1/responses', '/openai/v1/responses'],
    ['/chat/completions', '/openai/v1/chat/completions'],
    ['/v1/messages', '/anthropic/v1/messages'],
    ['/anthropic/v1/messages', '/anthropic/v1/messages']]) {
    const response = await fetch(base + path, { method: 'POST', body: '{}' });
    assert.equal(response.status, 200); assert.equal(await response.text(), expected);
  }
  const response = await fetch(base + '/v1/models', { headers: { 'anthropic-version': '2023-06-01' } });
  assert.equal(await response.text(), '/anthropic/v1/models');
});

test('management is unreachable and private headers do not cross the proxy', async t => {
  let calls = 0;
  const base = await setup(t, (req, res) => {
    calls++; res.setHeader('x-framerize-account', 'private');
    res.setHeader('set-cookie', 'private=value'); res.end(JSON.stringify(req.headers));
  });
  for (const path of ['/api/accounts', '/api/pool', '/api/run', '/api/settings', '/%2e%2e/api/accounts'])
    assert.equal((await fetch(base + path)).status, 404);
  assert.equal(calls, 0);
  const response = await fetch(base + '/responses', { method: 'POST', body: '{}',
    headers: { authorization: 'Bearer client-placeholder', 'x-framerize-account': 'private' } });
  const headers = await response.json();
  assert.equal(headers.authorization, undefined); assert.equal(headers['x-framerize-account'], undefined);
  assert.equal(response.headers.get('set-cookie'), null);
  assert.equal(response.headers.get('x-framerize-account'), null);
});

test('SSE arrives before upstream completion and cancellation closes upstream', async t => {
  let closed;
  const ended = new Promise(resolve => { closed = resolve; });
  const base = await setup(t, (req, res) => {
    res.writeHead(200, { 'content-type': 'text/event-stream' });
    res.write('data: first\n\n'); res.once('close', closed);
  });
  const response = await fetch(base + '/responses', { method: 'POST', body: '{}' });
  const reader = response.body.getReader();
  assert.match(new TextDecoder().decode((await reader.read()).value), /data: first/);
  await reader.cancel();
  await Promise.race([ended, new Promise((_, reject) => {
    const timer = setTimeout(() => reject(new Error('upstream was not cancelled')), 2000);
    timer.unref();
  })]);
});

test('concurrency and body bounds name the actual local limit', async t => {
  const held = [];
  const base = await setup(t, (req, res) => {
    held.push(res); res.writeHead(200); res.flushHeaders();
  }, { limit: 1, maxBody: 8 });
  const first = await fetch(base + '/responses', { method: 'POST', body: '{}' });
  const busy = await fetch(base + '/responses', { method: 'POST', body: '{}' });
  assert.equal(busy.status, 503); assert.match(await busy.text(), /1 active requests/);
  held[0].end('done'); assert.equal(await first.text(), 'done');
  const large = await fetch(base + '/responses', { method: 'POST', body: '123456789' });
  assert.equal(large.status, 413); assert.match(await large.text(), /8 bytes/);
  assert.equal(held.length, 1);
});

test('browser preflight permits SDK headers only on inference paths', async t => {
  const base = await setup(t, (_req, res) => res.end());
  const response = await fetch(base + '/v1/messages', { method: 'OPTIONS', headers: {
    'access-control-request-headers': 'anthropic-version,x-stainless-lang' } });
  assert.equal(response.status, 204);
  assert.equal(response.headers.get('access-control-allow-headers'), 'anthropic-version,x-stainless-lang');
  assert.equal((await fetch(base + '/api/accounts', { method: 'OPTIONS' })).status, 404);
});

test('chunked bodies obey the same size bound', async t => {
  const base = await setup(t, (_req, res) => { res.on('error', () => {}); }, { maxBody: 4 });
  const response = await new Promise((resolve, reject) => {
    const request = http.request(base + '/responses', { method: 'POST' }, resolve);
    request.on('error', reject); request.write('12345'); request.end();
  });
  assert.equal(response.statusCode, 413); response.resume();
});

test('upstream retry decisions reach SDKs without changing the error', async t => {
  const body = JSON.stringify({ error: { message: 'generation completion is unknown' } });
  const base = await setup(t, (_req, res) => {
    res.writeHead(502, { 'content-type': 'application/json', 'x-should-retry': 'false' });
    res.end(body);
  });
  const response = await fetch(base + '/v1/messages', { method: 'POST', body: '{}' });
  assert.equal(response.status, 502);
  assert.equal(response.headers.get('x-should-retry'), 'false');
  assert.match(response.headers.get('access-control-expose-headers'), /x-should-retry/);
  assert.equal(await response.text(), body);
});
