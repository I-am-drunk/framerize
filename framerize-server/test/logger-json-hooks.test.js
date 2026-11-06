import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/logger.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
const { createLogger } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

function capture() {
  const lines = [];
  return { lines, logger: createLogger({ stream: { write: line => lines.push(line) } }) };
}

function record(lines, event, level = 'info') {
  assert.equal(lines.length, 1);
  assert.ok(lines[0].endsWith('\n'));
  const parsed = JSON.parse(lines[0]);
  assert.equal(parsed.event, event);
  assert.equal(parsed.level, level);
  assert.ok(Number.isFinite(Date.parse(parsed.time)));
  return parsed;
}

test('a top-level serialization method cannot replace the log record', context => {
  const { logger, lines } = capture();
  let calls = 0;
  logger.warn('synthetic.event', { detail: 'kept', toJSON() {
    calls += 1;
    return { event: 'replaced', level: 'debug', time: 'replaced' };
  } });
  context.diagnostic(JSON.stringify({ hookCalls: calls, emitted: lines.map(JSON.parse) }));
  assert.equal(calls, 0, 'supplied serialization code must not run');
  assert.equal(record(lines, 'synthetic.event', 'warn').detail, 'kept');
});

test('a nested serialization method cannot bypass the string bound', context => {
  const { logger, lines } = capture();
  let calls = 0;
  logger.info('synthetic.bounded', { payload: { text: 'x'.repeat(2001), toJSON() {
    calls += 1;
    return 'x'.repeat(2001);
  } } });
  const parsed = record(lines, 'synthetic.bounded');
  context.diagnostic(JSON.stringify({ hookCalls: calls, payloadType: typeof parsed.payload,
    stringLength: typeof parsed.payload === 'string' ? parsed.payload.length : parsed.payload.text.length }));
  assert.equal(calls, 0);
  assert.deepEqual(parsed.payload, { text: 'x'.repeat(2000) });
});

test('an error serialization method cannot discard the log record', context => {
  const { logger, lines } = capture();
  let calls = 0;
  const error = Object.assign(new Error('synthetic failure'), { toJSON() {
    calls += 1;
    throw new Error('synthetic serialization rejection');
  } });
  logger.error('synthetic.failure', { error });
  context.diagnostic(JSON.stringify({ hookCalls: calls, emittedCount: lines.length }));
  assert.equal(calls, 0);
  assert.deepEqual(record(lines, 'synthetic.failure', 'error').error,
    { name: 'Error', message: 'synthetic failure' });
});

test('function values cannot run attached serialization methods', context => {
  const { logger, lines } = capture();
  let calls = 0;
  const method = Object.assign(() => {}, { toJSON() {
    calls += 1;
    return 'x'.repeat(2001);
  } });
  logger.info('synthetic.functions', { method, values: [method] });
  const parsed = record(lines, 'synthetic.functions');
  context.diagnostic(JSON.stringify({ hookCalls: calls, propertyPresent: Object.hasOwn(parsed, 'method'),
    arrayValueType: typeof parsed.values[0] }));
  assert.equal(calls, 0);
  assert.equal(Object.hasOwn(parsed, 'method'), false);
  assert.deepEqual(parsed.values, [null]);
});

test('ordinary data and noncallable toJSON fields retain JSON behavior', () => {
  const { logger, lines } = capture();
  const method = () => {};
  logger.info('synthetic.control', { token: 'synthetic-visible-token', toJSON: 'ordinary text',
    payload: { toJSON: 42, text: 'x'.repeat(2001), method }, values: [method, 7n, null] });
  const parsed = record(lines, 'synthetic.control');
  assert.equal(parsed.token, 'synthetic-visible-token');
  assert.equal(parsed.toJSON, 'ordinary text');
  assert.deepEqual(parsed.payload, { toJSON: 42, text: 'x'.repeat(2000) });
  assert.deepEqual(parsed.values, [null, '7', null]);
});
