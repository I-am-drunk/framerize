import test from 'node:test';
import assert from 'node:assert/strict';
import { createCastleMinter } from '../dist/castle.js';

function activeTimers() {
  return process.getActiveResourcesInfo().filter(type => type === 'Timeout').length;
}

test('successful mint clears referenced realm timers', async (t) => {
  const minter = createCastleMinter({ timeoutMs: 120_000, budgetMs: 8000 });
  t.after(() => minter.close());
  const before = activeTimers();
  const { token } = await minter.mint({ signal: AbortSignal.timeout(120_000) });
  assert.equal(typeof token, 'string');
  assert.ok(token.length >= 64);
  assert.equal(activeTimers(), before);
});

test('timed-out mint clears referenced realm timers', async (t) => {
  const minter = createCastleMinter({ timeoutMs: 50, budgetMs: 8000 });
  t.after(() => minter.close());
  const before = activeTimers();
  await assert.rejects(() => minter.mint({ signal: AbortSignal.timeout(120_000) }), /timeout/);
  assert.equal(activeTimers(), before);
});

test('mint rejects when closed without starting referenced timers', async () => {
  const minter = createCastleMinter({ timeoutMs: 120_000, budgetMs: 8000 });
  await minter.close();
  const before = activeTimers();
  await assert.rejects(() => minter.mint({ signal: AbortSignal.timeout(120_000) }), /closed/);
  assert.equal(activeTimers(), before);
});

test('sequential mints do not accumulate referenced realm timers', async (t) => {
  const minter = createCastleMinter({ timeoutMs: 120_000, budgetMs: 8000 });
  t.after(() => minter.close());
  const before = activeTimers();
  await minter.mint({ signal: AbortSignal.timeout(120_000) });
  assert.equal(activeTimers(), before);
  await minter.mint({ signal: AbortSignal.timeout(120_000) });
  assert.equal(activeTimers(), before);
});

test('concurrent mint rejects as busy without leaving referenced timers', async (t) => {
  const minter = createCastleMinter({ timeoutMs: 120_000, budgetMs: 8000 });
  t.after(() => minter.close());
  const before = activeTimers();
  const first = minter.mint({ signal: AbortSignal.timeout(120_000) });
  await assert.rejects(() => minter.mint({ signal: AbortSignal.timeout(120_000) }), /busy/);
  await first;
  assert.equal(activeTimers(), before);
});
