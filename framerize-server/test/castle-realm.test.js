import test from 'node:test';
import assert from 'node:assert/strict';
import { createCastleMinter } from '../dist/castle.js';

/** Active libuv timers; used to catch realm mint timers that outlive cleanup. */
function activeTimeoutHandles() {
  return process._getActiveHandles().filter((handle) => handle.constructor?.name === 'Timeout');
}

test('successful mint clears realm timers so handles match the pre-mint baseline', async (t) => {
  const minter = createCastleMinter({ timeoutMs: 120_000, budgetMs: 8000 });
  t.after(() => minter.close());
  const before = activeTimeoutHandles().length;
  const { token } = await minter.mint({ signal: AbortSignal.timeout(120_000) });
  assert.equal(typeof token, 'string');
  assert.ok(token.length >= 64);
  assert.equal(activeTimeoutHandles().length, before);
});

test('timed-out mint still clears realm timers and the watchdog', async (t) => {
  const minter = createCastleMinter({ timeoutMs: 50, budgetMs: 8000 });
  t.after(() => minter.close());
  const before = activeTimeoutHandles().length;
  await assert.rejects(() => minter.mint({ signal: AbortSignal.timeout(120_000) }), /timeout/);
  assert.equal(activeTimeoutHandles().length, before);
});

test('mint rejects when closed without leaving timers active', async (t) => {
  const minter = createCastleMinter({ timeoutMs: 120_000, budgetMs: 8000 });
  await minter.close();
  const before = activeTimeoutHandles().length;
  await assert.rejects(() => minter.mint({ signal: AbortSignal.timeout(120_000) }), /closed/);
  assert.equal(activeTimeoutHandles().length, before);
});
