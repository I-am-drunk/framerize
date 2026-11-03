import test from 'node:test';
import assert from 'node:assert/strict';
import { Readable } from 'node:stream';
import { encodeSse, parseSse, createDeadline } from '../src/sse.js';

async function collect(chunks) {
  const events = [];
  const buffers = chunks.map((chunk) => Buffer.from(chunk));
  for await (const event of parseSse(Readable.from(buffers))) events.push(event);
  return events;
}

test('SSE parser handles fragmented and coalesced events', async () => {
  const events = await collect([
    'event: response.output_text.delta\nda',
    'ta: {"text":"hel"}\n\nid: 7\ndata: lo\n\n: ping\n\n',
  ]);
  assert.deepEqual(events, [
    { event: 'response.output_text.delta', data: '{"text":"hel"}' },
    { event: 'message', data: 'lo', id: '7' },
  ]);
});

test('SSE encoder preserves multiline data', () => {
  assert.equal(encodeSse({ event: 'done', data: 'a\nb' }), 'event: done\ndata: a\ndata: b\n\n');
});

test('every byte split preserves UTF-8 and CRLF events', async () => {
  const source = Buffer.from('event: delta\r\ndata: {"text":"hé"}\r\n\r\n');
  for (let split = 1; split < source.length; split += 1) {
    const events = [];
    const body = Readable.from([source.subarray(0, split), source.subarray(split)]);
    for await (const event of parseSse(body)) events.push(event);
    assert.deepEqual(events, [{ event: 'delta', data: '{"text":"hé"}' }]);
  }
});

test('incomplete and data-free EOF frames do not dispatch', async () => {
  assert.deepEqual(await collect(['event: ignored\n\nid: 9\n\ndata: incomplete']), []);
});

test('CR lines and split CRLF boundaries preserve events', async () => {
  assert.deepEqual(await collect(['event: one\rdata: a\r\r', 'event: two\r\ndata: b\r', '\n\r\n']), [
    { event: 'one', data: 'a' }, { event: 'two', data: 'b' },
  ]);
});

test('encoder strips newlines from event names', () => {
  assert.equal(encodeSse({ event: 'one\r\ndata: injected', data: 'safe' }),
    'event: onedata: injected\ndata: safe\n\n');
});
