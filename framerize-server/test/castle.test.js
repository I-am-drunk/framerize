import test from 'node:test';
import assert from 'node:assert/strict';
import { setTimeout as delay } from 'node:timers/promises';
import { isDeepStrictEqual } from 'node:util';
import { createCastleMinter } from '../dist/castle.js';

function transportResources() {
  return process.getActiveResourcesInfo().filter(name => /TCP|Timeout/.test(name)).sort();
}

async function assertTransportClosed(expected, timeoutMs = 1000) {
  const deadline = performance.now() + timeoutMs;
  let actual = transportResources();
  while (!isDeepStrictEqual(actual, expected) && performance.now() < deadline) {
    await delay(10);
    actual = transportResources();
  }
  assert.deepEqual(actual, expected, 'Castle mint left TCP or timer resources');
}

test('Castle mint clears realm timers so the event loop can settle', { timeout: 30_000 }, async () => {
  const baseline = transportResources();
  const minter = createCastleMinter({ budgetMs: 8000, timeoutMs: 9000 });
  const { token } = await minter.mint({ signal: AbortSignal.timeout(25_000) });
  assert.equal(typeof token, 'string');
  assert.ok(token.length >= 64 && token.length <= 16_384);
  await assertTransportClosed(baseline);
  await minter.close();
});

test('Castle mint timeout still reaps realm timers', { timeout: 30_000 }, async () => {
  const baseline = transportResources();
  const minter = createCastleMinter({ budgetMs: 8000, timeoutMs: 1 });
  await assert.rejects(minter.mint({ signal: AbortSignal.timeout(5000) }), { message: 'timeout' });
  await assertTransportClosed(baseline);
  await minter.close();
});

test('Castle minter rejects overlapping mints and use after close', async () => {
  const minter = createCastleMinter({ budgetMs: 8000, timeoutMs: 9000 });
  const first = minter.mint({ signal: AbortSignal.timeout(25_000) });
  await assert.rejects(minter.mint({ signal: AbortSignal.timeout(1000) }), { message: 'busy' });
  await first;
  await minter.close();
  await assert.rejects(minter.mint({ signal: AbortSignal.timeout(1000) }), { message: 'closed' });
});
