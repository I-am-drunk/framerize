import assert from 'node:assert/strict';
import test from 'node:test';
import { createServerMinter } from '../dist/castle-loader.js';

test('the server minter preserves the supplied account device identity', { timeout: 10_000 }, async context => {
  const signal = new AbortController().signal;
  const minter = await createServerMinter(signal);
  context.after(() => minter.close());
  const cuid = 'synthetic-existing-account-device';
  const result = await minter.mint({ signal, cuid });
  assert.equal(result.cuid, cuid);
  assert.match(result.token, /^cNz8rPYH\|[A-Za-z0-9+/]+={0,2}$/);
});
