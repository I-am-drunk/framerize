import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { parseConfig } from '../src/config.js';
import { createServer } from '../src/server.js';

async function fixture(t, fetchImpl, accounts = []) {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-'));
  const file = path.join(directory, 'accounts.json');
  await writeFile(file, JSON.stringify(accounts));
  const app = await createServer(parseConfig({
    port: 0, accounts: file, upstream: 'http://upstream.test',
  }), { fetchImpl });
  t.after(() => app.close());
  const address = await app.listen();
  return `http://127.0.0.1:${address.port}`;
}

test('health and unknown routes do not call upstream', async (t) => {
  let calls = 0;
  const base = await fixture(t, () => { calls += 1; });
  assert.equal((await fetch(`${base}/health`)).status, 200);
  assert.equal((await fetch(`${base}/missing`)).status, 404);
  assert.equal(calls, 0);
});

test('responses proxy preserves SSE and strips caller credentials', async (t) => {
  let seen;
  const base = await fixture(t, async (url, options) => {
    seen = { url: url.href, headers: options.headers };
    return new Response('data: {"ok":true}\n\n', {
      status: 200, headers: { 'content-type': 'text/event-stream' },
    });
  });
  const response = await fetch(`${base}/v1/responses`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', cookie: 'DO_NOT_FORWARD' },
    body: JSON.stringify({ model: 'test', input: 'hello' }),
  });
  assert.equal(await response.text(), 'data: {"ok":true}\n\n');
  assert.equal(seen.url, 'http://upstream.test/v1/responses');
  assert.equal(seen.headers.cookie, undefined);
});

test('long streams preserve every chunk', async (t) => {
  const chunks = Array.from({ length: 2000 }, (_, index) => `data: ${index}\n\n`);
  const base = await fixture(t, async () => new Response(ReadableStream.from(chunks)));
  const response = await fetch(`${base}/v1/chat/completions`, {
    method: 'POST', body: '{}',
  });
  assert.equal(await response.text(), chunks.join(''));
});

test('upstream errors preserve status and body', async (t) => {
  const base = await fixture(t, async () => new Response('rate limited', {
    status: 429, headers: { 'retry-after': '10' },
  }));
  const response = await fetch(`${base}/v1/responses`, { method: 'POST', body: '{}' });
  assert.equal(response.status, 429);
  assert.equal(response.headers.get('retry-after'), '10');
  assert.equal(await response.text(), 'rate limited');
});

test('oversized request returns 413 without upstream call', async (t) => {
  let calls = 0;
  const base = await fixture(t, () => { calls += 1; });
  const response = await fetch(`${base}/v1/responses`, {
    method: 'POST', body: 'x'.repeat(16 * 1024 * 1024 + 1),
  });
  assert.equal(response.status, 413);
  assert.equal(calls, 0);
});

test('close before listen and repeated close are safe', async () => {
  const app = await createServer(parseConfig({ port: 0 }));
  await app.close();
  await app.close();
  await app.listen();
  await Promise.all([app.close(), app.close()]);
});
