import test from 'node:test';
import assert from 'node:assert/strict';
import { idleTimeout } from '../src/sse.js';
import { createDeadline } from '../src/upstream.js';

test('return rejection during idle cleanup is not unhandled', async () => {
  const iterator = {
    next: () => new Promise(() => {}),
    return: () => Promise.reject(new Error('cannot close')),
    [Symbol.asyncIterator]() { return this; },
  };
  await assert.rejects(async () => {
    for await (const chunk of idleTimeout(iterator, 5)) void chunk;
  }, /idle timeout/);
});

test('idle timeout rejects without waiting for source closure', async () => {
  const source = new ReadableStream({ pull() { return new Promise(() => {}); } });
  const started = Date.now();
  await assert.rejects(async () => {
    for await (const chunk of idleTimeout(source, 20)) void chunk;
  }, /idle timeout/);
  assert.ok(Date.now() - started < 500);
});

test('periodic chunks reset idle timeout', async () => {
  async function* source() {
    yield 'one';
    await new Promise((resolve) => setTimeout(resolve, 10));
    yield 'two';
  }
  const values = [];
  for await (const value of idleTimeout(source(), 100)) values.push(value);
  assert.deepEqual(values, ['one', 'two']);
});

test('deadline aborts an independent client signal', async () => {
  const client = new AbortController();
  const deadline = createDeadline(client.signal, 10);
  const aborted = new Promise((resolve) => deadline.signal.addEventListener('abort', resolve, { once: true }));
  await aborted;
  assert.equal(deadline.signal.aborted, true);
  assert.equal(client.signal.aborted, false);
  assert.equal(deadline.timedOut(), true);
  deadline.clear();
});
