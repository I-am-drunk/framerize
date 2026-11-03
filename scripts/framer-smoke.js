import assert from 'node:assert/strict';
import http from 'node:http';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { parseConfig } from '../src/config.js';
import { createServer } from '../src/server.js';

const events = [
  { id: 'chatcmpl-fixture', object: 'chat.completion.chunk',
    choices: [{ delta: { content: 'hello' } }] },
].map((event) => `data: ${JSON.stringify(event)}\n\n`);
events.push('data: [DONE]\n\n');
let envelope;
let headers;
const upstream = http.createServer((request, response) => {
  let body = '';
  request.setEncoding('utf8');
  request.on('data', (chunk) => { body += chunk; });
  request.on('end', () => {
    envelope = JSON.parse(body);
    headers = request.headers;
    response.writeHead(200, { 'content-type': envelope.request.stream
      ? 'text/event-stream' : 'application/json' });
    if (!envelope.request.stream) {
      response.end(JSON.stringify({ id: 'chatcmpl-fixture',
        choices: [{ message: { role: 'assistant', content: 'hello' } }] }));
      return;
    }
    response.write(events[0]);
    setTimeout(() => response.end(events[1]), 25);
  });
});
await new Promise((resolve) => upstream.listen(0, '127.0.0.1', resolve));
const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-framer-smoke-'));
const accounts = path.join(directory, 'accounts.json');
await writeFile(accounts, JSON.stringify([{
  id: 'fixture', provider: 'framer', projectId: 'fixture-project',
  upstream: `http://127.0.0.1:${upstream.address().port}`,
  authorizationEnv: 'FRAMER_SMOKE_JWT', castleEnv: 'FRAMER_SMOKE_CASTLE',
}]));
process.env.FRAMER_SMOKE_JWT = 'fixture-jwt';
process.env.FRAMER_SMOKE_CASTLE = 'fixture-castle';
const app = await createServer(parseConfig({ port: 0, accounts }));
try {
  const address = await app.listen();
  const base = `http://127.0.0.1:${address.port}`;
  const streamed = await fetch(`${base}/v1/chat/completions`, {
    method: 'POST', body: JSON.stringify({ model: 'fixture', stream: true,
      messages: [{ role: 'user', content: 'hello' }] }),
  });
  const text = await streamed.text();
  assert.match(text, /"content":"hello"/);
  assert.match(text, /data: \[DONE\]/);
  assert.equal(envelope.userPrompt, 'hello');
  assert.equal(headers['x-requested-by'], 'Framer');
  assert.equal(headers['content-type'], 'application/json; charset=utf-8');
  assert.match(envelope.sessionId, /^[A-Za-z0-9]{9}$/);
  assert.match(envelope.turnId, /^[A-Za-z0-9]{9}$/);
  assert.deepEqual(envelope.request.messages, [{ role: 'user', content: 'hello' }]);
  assert.equal(envelope.request.model, 'fixture');
  assert.equal(envelope.request.stream, true);
  assert.equal(envelope.request.input, undefined);
  const complete = await fetch(`${base}/v1/chat/completions`, {
    method: 'POST', body: JSON.stringify({ model: 'fixture',
      messages: [{ role: 'user', content: 'hello' }] }),
  });
  const result = await complete.json();
  assert.equal(result.choices[0].message.content, 'hello');
  console.log('Framer envelope and chat streaming smoke passed');
} finally {
  await app.close();
  await new Promise((resolve) => upstream.close(resolve));
  delete process.env.FRAMER_SMOKE_JWT;
  delete process.env.FRAMER_SMOKE_CASTLE;
}
