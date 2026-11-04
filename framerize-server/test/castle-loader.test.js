import assert from 'node:assert/strict';
import test from 'node:test';
import { createServerMinter } from '../dist/castle-loader.js';

test('createServerMinter propagates abort before returning a minter', async () => {
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(() => createServerMinter(controller.signal), { name: 'AbortError' });
});

test('createServerMinter returns a minter that mints encoded tokens', async context => {
  const minter = await createServerMinter(new AbortController().signal);
  context.after(() => minter.close());
  const { token } = await minter.mint({ signal: AbortSignal.timeout(9000) });
  assert.match(token, /^cNz8rPYH\|[A-Za-z0-9+/]+={0,2}$/);
});
