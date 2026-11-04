import test from 'node:test';
import assert from 'node:assert/strict';
import { createCastleMinter } from '../dist/castle.js';

test('mint rejects promptly when the caller signal is already aborted', async () => {
  const minter = createCastleMinter({ timeoutMs: 60_000 });
  const controller = new AbortController();
  controller.abort(new Error('synthetic-abort'));
  const started = performance.now();
  await assert.rejects(minter.mint({ signal: controller.signal }), /synthetic-abort/);
  assert.ok(performance.now() - started < 2000, 'aborted mint should not wait for the Castle watchdog');
});

test('mint aborts when the caller signal fires during an active mint', async () => {
  const minter = createCastleMinter({ timeoutMs: 60_000 });
  const controller = new AbortController();
  const pending = minter.mint({ signal: controller.signal });
  controller.abort(new Error('synthetic-abort'));
  await assert.rejects(pending, /synthetic-abort/);
});
