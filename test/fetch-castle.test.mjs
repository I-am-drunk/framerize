import assert from 'node:assert/strict';
import test from 'node:test';
import { readRevision } from '../scripts/fetch-castle.mjs';

test('accepts a full Castle commit', () => {
  assert.equal(readRevision('5835f3db681c9e26b5213238f4714795bd29caee\n'),
    '5835f3db681c9e26b5213238f4714795bd29caee');
});

test('rejects a branch name or short commit', () => {
  assert.throws(() => readRevision('main'), /40-character/);
  assert.throws(() => readRevision('5835f3db'), /40-character/);
});
