import test from 'node:test';
import assert from 'node:assert/strict';
import { idleTimeout } from '../src/sse.js';

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
