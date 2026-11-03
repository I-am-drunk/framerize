import test from 'node:test';
import assert from 'node:assert/strict';
import { openUpstream } from '../src/upstream.js';

test('an absent upstream endpoint fails clearly', async () => {
  await assert.rejects(openUpstream({ config: {}, request: { url: '/v1/responses' },
    body: Buffer.from('{}') }), /no upstream endpoint configured/);
});
