import test from 'node:test';
import assert from 'node:assert/strict';
import { Writable } from 'node:stream';
import { createLogger } from '../src/logger.js';

function captureLogger(level = 'info') {
  let output = '';
  const stream = new Writable({
    write(chunk, encoding, callback) { output += chunk.toString(); callback(); },
  });
  return { logger: createLogger({ stream, level }), read: () => output };
}

test('structured logs redact bearer tokens and credential-shaped fields', () => {
  const { logger, read } = captureLogger();
  logger.info('request.failed', {
    authorization: 'Bearer fixture-secret',
    cookie: 'session=fixture-secret',
    nested: { token: 'fixture-secret', safe: 'retained' },
    error: new Error('upstream rejected Bearer fixture-secret'),
  });
  const output = read();
  assert.equal(output.includes('fixture-secret'), false);
  const line = JSON.parse(output);
  assert.equal(line.nested.safe, 'retained');
  assert.equal(line.level, 'info');
});

test('log level suppresses lower-priority records', () => {
  const { logger, read } = captureLogger('error');
  logger.info('hidden');
  logger.error('visible', { requestId: 'fixture' });
  assert.equal(read().includes('hidden'), false);
  assert.equal(read().includes('visible'), true);
});

test('silent logging emits nothing and cyclic fields are bounded', () => {
  const silent = captureLogger('silent');
  silent.logger.error('hidden', { token: 'fixture-secret' });
  assert.equal(silent.read(), '');
  const { logger, read } = captureLogger();
  const fields = { requestId: 'fixture' };
  fields.circular = fields;
  logger.info('cycle', fields);
  const line = JSON.parse(read());
  assert.equal(line.circular.circular, '[circular]');
});
