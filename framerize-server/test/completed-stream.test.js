import test from 'node:test';
import assert from 'node:assert/strict';
import { streamCompletedReply } from '../dist/compat/completed-stream.js';
import { parseCompletedReply } from '../dist/compat/completed-reply.js';
import { ProviderError } from '../dist/errors.js';
import { deferred } from './helpers/fixtures.js';

async function collect(generator) {
  const chunks = [];
  for await (const chunk of generator) chunks.push(chunk);
  return chunks;
}

function parseEvents(chunks) {
  return Buffer.concat(chunks).toString().split('\n\n').flatMap(frame => {
    if (frame.startsWith(': keepalive')) return [{ kind: 'keepalive' }];
    const data = frame.split('\n').find(line => line.startsWith('data: '));
    return data ? [{ kind: 'event', value: JSON.parse(data.slice(6)) }] : [];
  });
}

function eventsOnly(chunks) {
  return parseEvents(chunks).filter(item => item.kind === 'event').map(item => item.value);
}

const messagesBody = { id: 'msg', type: 'message', role: 'assistant', model: 'fixture',
  stop_reason: 'end_turn', stop_sequence: null, usage: { input_tokens: 3, output_tokens: 5 },
  content: [{ type: 'text', text: 'hello' }] };
const responsesBody = { id: 'resp', object: 'response', model: 'fixture', status: 'completed', created_at: 1,
  usage: { input_tokens: 3, output_tokens: 5, total_tokens: 8 },
  output: [{ type: 'message', role: 'assistant', id: 'out', status: 'completed',
    content: [{ type: 'output_text', text: 'hello' }] }] };

for (const [protocol, body, terminal] of [
  ['messages', messagesBody, 'message_stop'],
  ['responses', responsesBody, 'response.completed'],
]) {
  test(`streamCompletedReply emits ${terminal} after the upstream reply resolves`, async () => {
    const reply = parseCompletedReply(protocol, Buffer.from(JSON.stringify(body)));
    const pending = Promise.resolve({ status: 200, reply });
    const received = eventsOnly(await collect(streamCompletedReply(pending, protocol,
      new AbortController().signal, 5000)));
    assert.equal(received.at(-1).type, terminal);
  });
}

test('streamCompletedReply yields keepalives until the upstream reply is ready', { timeout: 1000 }, async context => {
  const gate = deferred();
  const reply = parseCompletedReply('messages', Buffer.from(JSON.stringify(messagesBody)));
  const pending = gate.promise.then(() => ({ status: 200, reply }));
  const generator = streamCompletedReply(pending, 'messages', context.signal, 15);
  const first = await generator.next();
  assert.equal(first.value.toString(), ': keepalive\n\n');
  const second = await generator.next();
  assert.equal(second.value.toString(), ': keepalive\n\n');
  gate.resolve();
  const tail = [];
  for await (const chunk of generator) tail.push(chunk);
  const events = parseEvents([first.value, second.value, ...tail]);
  assert.equal(events.at(-1).value.type, 'message_stop');
});

for (const protocol of ['messages', 'responses']) {
  test(`missing completed reply becomes a single ${protocol} stream error`, async () => {
    const pending = Promise.resolve({ status: 502, headers: {}, body: Buffer.alloc(0) });
    const events = eventsOnly(await collect(streamCompletedReply(pending, protocol,
      new AbortController().signal, 5000)));
    assert.equal(events.length, 1);
    assert.equal(events[0].type, 'error');
    if (protocol === 'messages') {
      assert.equal(events[0].error.code, 'upstream_http_error');
      assert.equal(events[0].error.type, 'api_error');
    } else {
      assert.equal(events[0].code, 'upstream_http_error');
      assert.equal(events[0].message, 'Upstream request failed');
    }
  });
}

test('ProviderError status maps to messages authentication_error', async () => {
  const pending = Promise.reject(new ProviderError('denied', {
    status: 401, code: 'upstream_http_error', retryable: false,
  }));
  const events = eventsOnly(await collect(streamCompletedReply(pending, 'messages',
    new AbortController().signal, 5000)));
  assert.equal(events[0].error.type, 'authentication_error');
  assert.equal(events[0].error.code, 'upstream_http_error');
});

test('unknown failures use upstream_error on the responses stream', async () => {
  const pending = Promise.reject(new Error('synthetic-private'));
  const events = eventsOnly(await collect(streamCompletedReply(pending, 'responses',
    new AbortController().signal, 5000)));
  assert.equal(events[0].code, 'upstream_error');
  assert.equal(JSON.stringify(events[0]).includes('synthetic-private'), false);
});

test('abort while waiting stops without replaying a terminal event', { timeout: 1000 }, async () => {
  const controller = new AbortController();
  const pending = new Promise(() => {});
  const generator = streamCompletedReply(pending, 'messages', controller.signal, 5000);
  try {
    assert.equal((await generator.next()).value.toString(), ': keepalive\n\n');
    const waiting = generator.next();
    controller.abort();
    await assert.rejects(waiting, { name: 'AbortError' });
    assert.deepEqual(await generator.next(), { value: undefined, done: true });
  } finally {
    controller.abort();
    await generator.return();
  }
});
