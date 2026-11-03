import test from 'node:test';
import assert from 'node:assert/strict';
import { Writable } from 'node:stream';
import { createLogger } from '../src/logger.js';

test('logger redacts nested credentials and bearer errors', () => {
  let output = '';
  const stream = new Writable({ write(chunk, encoding, callback) {
    output += chunk.toString();
    callback();
  } });
  const logger = createLogger({ stream });
  logger.error('failure', {
    nested: { access_token: 'FAKE_TOKEN', authorization: 'FAKE_AUTH' },
    error: new Error('Bearer FAKE_SECRET'),
  });
  assert.equal(output.includes('FAKE_TOKEN'), false);
  assert.equal(output.includes('FAKE_AUTH'), false);
  assert.equal(output.includes('FAKE_SECRET'), false);
  assert.equal(output.includes('failure'), true);
});
