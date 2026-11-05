import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { once, getEventListeners } from 'node:events';
import { Readable } from 'node:stream';
import { createProvider } from '../dist/provider.js';

async function forward(context, connectionEntries) {
  const calls = [];
  const server = http.createServer((request, response) => {
    calls.push(request.headers);
    request.resume();
    response.end('synthetic response');
  });
  let provider;
  context.after(async () => {
    await provider?.close();
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  provider = createProvider({ upstream: `http://127.0.0.1:${server.address().port}`,
    authorization: 'synthetic-provider' });
  const controller = new AbortController();
  const result = await provider.forward({ path: '/v1/responses',
    body: Readable.from([Buffer.from('{}')]), signal: controller.signal,
    headers: { 'OpenAI-Beta': 'synthetic-openai', 'Anthropic-Beta': 'synthetic-anthropic',
      'Anthropic-Version': '2023-06-01', Authorization: 'Bearer synthetic-client',
      Cookie: 'synthetic-cookie', ...Object.fromEntries(connectionEntries) } });
  assert.equal(result.status, 200);
  const chunks = [];
  for await (const chunk of result.body) chunks.push(chunk);
  assert.equal(Buffer.concat(chunks).toString(), 'synthetic response');
  await provider.close();
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].authorization, 'Bearer synthetic-provider');
  assert.equal(calls[0].cookie, undefined);
  assert.equal(calls[0]['anthropic-version'], '2023-06-01');
  return calls[0];
}

for (const reverse of [false, true]) {
  test(`Connection combines separate nominations, reverse=${reverse}`,
    { timeout: 5000 }, async context => {
      const entries = [['Connection', ['OpenAI-Beta']], ['connection', ' Anthropic-Beta ']];
      if (reverse) entries.reverse();
      const received = await forward(context, entries);
      assert.equal(received['openai-beta'], undefined);
      assert.equal(received['anthropic-beta'], undefined);
    });
}

test('unrelated Connection entries leave allowed end-to-end headers intact',
  { timeout: 5000 }, async context => {
    for (const entries of [[], [['Connection', 'keep-alive'], ['connection', 'close']]]) {
      const received = await forward(context, entries);
      assert.equal(received['openai-beta'], 'synthetic-openai');
      assert.equal(received['anthropic-beta'], 'synthetic-anthropic');
    }
  });

for (const names of [['Connection', 'connection'], ['connection', 'CONNECTION'],
  ['CONNECTION', 'Connection']]) {
  for (const nominatedFirst of [true, false]) {
    test(`Connection nominations with ${names.join('/')} and nominatedFirst=${nominatedFirst}`,
      { timeout: 5000 }, async context => {
        const values = [' OpenAI-Beta , ANTHROPIC-BETA ', 'keep-alive'];
        if (!nominatedFirst) values.reverse();
        const received = await forward(context, names.map((name, index) => [name, values[index]]));
        assert.equal(received['openai-beta'], undefined);
        assert.equal(received['anthropic-beta'], undefined);
      });
  }
}
