import test from 'node:test';
import assert from 'node:assert/strict';
import { Writable } from 'node:stream';
import { createLogger } from '../dist/logger.js';

function captureLogger(level = 'info') {
  let output = '';
  const stream = new Writable({
    write(chunk, encoding, callback) { output += chunk.toString(); callback(); },
  });
  return { logger: createLogger({ stream, level }), read: () => output };
}

test('structured logs preserve explicitly supplied fields and error details', () => {
  const { logger, read } = captureLogger();
  logger.info('request.failed', {
    authorization: 'Bearer fixture-secret',
    cookie: 'session=fixture-secret',
    nested: { token: 'fixture-secret', safe: 'retained' },
    error: new Error('upstream rejected Bearer fixture-secret'),
    prompt: 'Explain this synthetic example',
    sessionCount: 3,
  });
  const output = read();
  assert.equal(output.includes('fixture-secret'), true);
  const line = JSON.parse(output);
  assert.equal(line.nested.safe, 'retained');
  assert.equal(line.prompt, 'Explain this synthetic example');
  assert.equal(line.sessionCount, 3);
  assert.deepEqual(line.error, { name: 'Error', message: 'upstream rejected Bearer fixture-secret' });
  assert.equal(line.level, 'info');
});

test('large supplied payloads and errors still have size limits', () => {
  const { logger, read } = captureLogger();
  logger.info('payload', {
    input: 'x'.repeat(4000),
    messages: Array.from({ length: 200 }, (_, index) => index),
    error: new Error('x'.repeat(4000)),
  });
  const line = JSON.parse(read());
  assert.equal(line.input, 'x'.repeat(2000));
  assert.equal(line.messages.length, 100);
  assert.equal(line.error.message, 'x'.repeat(2000));
});

test('log level suppresses lower-priority records', () => {
  const { logger, read } = captureLogger('error');
  logger.info('hidden');
  logger.error('visible', { requestId: 'fixture' });
  assert.equal(read().includes('hidden'), false);
  assert.equal(read().includes('visible'), true);
});

test('logging does not interrupt requests when the output stream throws', () => {
  const logger = createLogger({ stream: { write() { throw new Error('synthetic output failure'); } } });
  assert.doesNotThrow(() => logger.info('request.finished', { status: 200 }));
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
