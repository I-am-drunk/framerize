import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import net from 'node:net';
import { parseConfig } from '../src/config.js';
import { createServer } from '../src/server.js';

async function fixture(t, fetchImpl, accounts = [], options = {}) {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-'));
  const file = path.join(directory, 'accounts.json');
  await writeFile(file, JSON.stringify(accounts));
  const app = await createServer(parseConfig({
    port: 0, accounts: file, upstream: 'http://upstream.test',
  }), { fetchImpl, ...options });
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
  const accounts = await fixture(t, () => {}, [
    { id: 'one', model: 'gpt-test', sessionEnv: 'TEST_SESSION' },
    { id: 'two', provider: 'framer', projectId: 'project', sessionFile: '/private/session' },
  ]);
  const response = await fetch(`${accounts}/v1/accounts`);
  assert.deepEqual(await response.json(), { accounts: [
    { id: 'one', model: 'gpt-test', credential: 'environment' },
    { id: 'two', credential: 'file' },
  ] });
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

test('account base paths join without duplicate slashes', async (t) => {
  let destination;
  const base = await fixture(t, async (url) => {
    destination = url.href;
    return new Response('ok');
  }, [{ id: 'one', upstream: 'http://upstream.test/openai/' }]);
  const response = await fetch(`${base}/v1/responses`, {
    method: 'POST', headers: { 'x-framer-account': 'one' }, body: '{}',
  });
  assert.equal(response.status, 200);
  assert.equal(destination, 'http://upstream.test/openai/v1/responses');
});

test('Framer inference uses the observed request headers', async (t) => {
  let headers;
  const base = await fixture(t, async (url, options) => {
    headers = options.headers;
    return new Response('{}', { headers: { 'content-type': 'application/json' } });
  }, [{ id: 'framer', provider: 'framer', projectId: 'project',
    authorizationEnv: 'TEST_JWT', castleEnv: 'TEST_CASTLE' }]);
  process.env.TEST_JWT = 'fixture';
  process.env.TEST_CASTLE = 'fixture';
  const response = await fetch(`${base}/v1/responses`, {
    method: 'POST', body: JSON.stringify({ model: 'test', input: 'hello' }),
  });
  assert.equal(response.status, 200);
  assert.equal(headers['x-requested-by'], 'Framer');
  assert.equal(headers['content-type'], 'application/json; charset=utf-8');
});

test('direct Framer accounts use the default API without a base URL', async (t) => {
  let destination;
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-'));
  const accounts = path.join(directory, 'accounts.json');
  await writeFile(accounts, JSON.stringify([{ id: 'framer', provider: 'framer',
    projectId: 'project', authorizationEnv: 'TEST_JWT', castleEnv: 'TEST_CASTLE' }]));
  const app = await createServer(parseConfig({ port: 0, accounts }), {
    fetchImpl: async (url) => {
      destination = url.href;
      return new Response('{}', { headers: { 'content-type': 'application/json' } });
    },
  });
  t.after(() => app.close());
  const address = await app.listen();
  process.env.TEST_JWT = 'fixture';
  process.env.TEST_CASTLE = 'fixture';
  const response = await fetch(`http://127.0.0.1:${address.port}/v1/responses`, {
    method: 'POST', body: '{}',
  });
  assert.equal(response.status, 200);
  assert.equal(destination, 'https://api.framer.com/ai/v3/openai/responses/');
});

test('streamed response failures close the downstream connection', async (t) => {
  const base = await fixture(t, async () => new Response(
    'data: {"type":"response.failed","response":{"error":{"message":"rejected"}}}\n\n',
    { headers: { 'content-type': 'application/json' } },
  ));
  const socket = net.connect(Number(new URL(base).port), '127.0.0.1');
  let received = '';
  socket.on('data', (chunk) => { received += chunk; });
  const closed = new Promise((resolve) => socket.once('close', resolve));
  await new Promise((resolve) => socket.once('connect', resolve));
  socket.end('POST /v1/responses HTTP/1.1\r\nHost: local\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}');
  await closed;
  assert.doesNotMatch(received, /\r\n0\r\n\r\n$/);
});

test('malformed Responses JSON returns a clean 502', async (t) => {
  const base = await fixture(t, async () => new Response('data: {broken\n\n', {
    headers: { 'content-type': 'application/json' },
  }));
  const response = await fetch(`${base}/v1/responses`, { method: 'POST', body: '{}' });
  assert.equal(response.status, 502);
  assert.match(await response.text(), /invalid JSON/);
});

test('invalid Responses events return a clean 502', async (t) => {
  const base = await fixture(t, async () => new Response('data: null\n\n', {
    headers: { 'content-type': 'application/json' },
  }));
  const response = await fetch(`${base}/v1/responses`, { method: 'POST', body: '{}' });
  assert.equal(response.status, 502);
  assert.match(await response.text(), /invalid response event/);
});

test('fragmented failure events abort without corrupting earlier chunks', async (t) => {
  const events = ['data: {"type":"response.output_text.delta","delta":"ok"}\n\n',
    'data: {"type":"response.failed","response":{}}\n\n'].join('');
  const chunks = Array.from(events, (character) => Buffer.from(character));
  const base = await fixture(t, async () => new Response(ReadableStream.from(chunks), {
    headers: { 'content-type': 'application/json' },
  }));
  await assert.rejects(fetch(`${base}/v1/responses`, { method: 'POST', body: '{}' })
    .then((response) => response.text()), /terminated|other side closed|fetch failed/);
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

test('body at the configured limit is accepted', async (t) => {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-'));
  const app = await createServer(parseConfig({ port: 0, upstream: 'http://upstream.test', maxBodyBytes: 2 }), {
    fetchImpl: async () => new Response('ok'),
  });
  t.after(() => app.close());
  await app.listen();
  const base = `http://127.0.0.1:${app.server.address().port}`;
  assert.equal((await fetch(`${base}/v1/responses`, { method: 'POST', body: '{}' })).status, 200);
  assert.equal((await fetch(`${base}/v1/responses`, { method: 'POST', body: 'xxx' })).status, 413);
});

test('unknown account returns 404 without upstream call', async (t) => {
  let calls = 0;
  const base = await fixture(t, () => { calls += 1; }, [{ id: 'known' }]);
  const response = await fetch(`${base}/v1/responses`, {
    method: 'POST', headers: { 'x-framer-account': 'unknown' }, body: '{}',
  });
  assert.equal(response.status, 404);
  assert.equal(calls, 0);
});

test('close before listen and repeated close are safe', async () => {
  const app = await createServer(parseConfig({ port: 0 }));
  await app.close();
  await app.close();
  await app.listen();
  await Promise.all([app.close(), app.close()]);
});

test('Framer chat preserves native streaming events', async (t) => {
  let envelope;
  const events = [
    { id: 'chatcmpl-1', object: 'chat.completion.chunk', choices: [
      { delta: { content: 'hello' } },
    ] },
  ].map((event) => `data: ${JSON.stringify(event)}\n\n`).join('');
  const base = await fixture(t, async (url, options) => {
    envelope = JSON.parse(options.body);
    return new Response(events, { headers: { 'content-type': 'text/event-stream' } });
  }, [{ id: 'framer', provider: 'framer', projectId: 'project',
    authorizationEnv: 'TEST_JWT', castleEnv: 'TEST_CASTLE' }]);
  process.env.TEST_JWT = 'fixture';
  process.env.TEST_CASTLE = 'fixture';
  const response = await fetch(`${base}/v1/chat/completions`, {
    method: 'POST', body: JSON.stringify({ model: 'gpt-test', messages: [{ role: 'user', content: 'hi' }], stream: true }),
  });
  const text = await response.text();
  assert.match(text, /"content":"hello"/);
  assert.equal(text, events);
  assert.equal(envelope.request.messages[0].role, 'user');
  assert.equal(envelope.request.input, undefined);
});

test('Framer non-streaming chat returns the native completion', async (t) => {
  const base = await fixture(t, async () => new Response(JSON.stringify({
    id: 'chatcmpl-1', created: 1, choices: [
      { message: { role: 'assistant', content: 'hello' } },
    ], usage: { prompt_tokens: 2, completion_tokens: 1, total_tokens: 3 },
  }), { headers: { 'content-type': 'application/json' } }), [
    { id: 'framer', provider: 'framer', projectId: 'project',
      authorizationEnv: 'TEST_JWT', castleEnv: 'TEST_CASTLE' },
  ]);
  process.env.TEST_JWT = 'fixture';
  process.env.TEST_CASTLE = 'fixture';
  const response = await fetch(`${base}/v1/chat/completions`, {
    method: 'POST', body: JSON.stringify({ model: 'gpt-test', messages: [{ role: 'user', content: 'hi' }] }),
  });
  const result = await response.json();
  assert.equal(result.choices[0].message.content, 'hello');
  assert.equal(result.usage.total_tokens, 3);
});

test('Framer chat streams native tool calls and accepts their results', async (t) => {
  const envelopes = [];
  const chunks = [
    { id: 'chatcmpl-tool', object: 'chat.completion.chunk', choices: [{ delta: {
      role: 'assistant', tool_calls: [{ index: 0, id: 'call1', type: 'function',
        function: { name: 'weather', arguments: '' } }],
    } }] },
    { id: 'chatcmpl-tool', object: 'chat.completion.chunk', choices: [{ delta: {
      tool_calls: [{ index: 0, function: { arguments: '{"city":"Paris"}' } }],
    } }] },
    { id: 'chatcmpl-tool', object: 'chat.completion.chunk', choices: [{ delta: {}, finish_reason: 'tool_calls' }] },
  ];
  const events = chunks.map((chunk) => `data: ${JSON.stringify(chunk)}\n\n`).join('') + 'data: [DONE]\n\n';
  const base = await fixture(t, async (url, options) => {
    envelopes.push(JSON.parse(options.body));
    return new Response(events, { headers: { 'content-type': 'text/event-stream' } });
  }, [{ id: 'framer', provider: 'framer', projectId: 'project',
    authorizationEnv: 'TEST_JWT', castleEnv: 'TEST_CASTLE' }]);
  process.env.TEST_JWT = 'fixture';
  process.env.TEST_CASTLE = 'fixture';
  const messages = [{ role: 'user', content: 'Check Paris weather' }];
  const tools = [{ type: 'function', function: { name: 'weather', parameters: { type: 'object' } } }];
  const response = await fetch(`${base}/v1/chat/completions`, {
    method: 'POST', body: JSON.stringify({ model: 'gpt-test', stream: true, messages, tools }),
  });
  assert.equal(await response.text(), events);
  messages.push({ role: 'assistant', content: null, tool_calls: [{ id: 'call1', type: 'function',
    function: { name: 'weather', arguments: '{"city":"Paris"}' } }] });
  messages.push({ role: 'tool', tool_call_id: 'call1', content: 'sunny' });
  const result = await fetch(`${base}/v1/chat/completions`, {
    method: 'POST', body: JSON.stringify({ model: 'gpt-test', stream: true, messages, tools }),
  });
  assert.equal(await result.text(), events);
  assert.deepEqual(envelopes[0].request.messages, messages.slice(0, 1));
  assert.deepEqual(envelopes[1].request.messages, messages);
  assert.deepEqual(envelopes[1].request.tools, tools);
});
