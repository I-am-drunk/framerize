import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/logger.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { createLogger } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);
const methods = ['debug', 'info', 'warn', 'error'];

test('suppressed log records never enumerate or evaluate supplied fields', () => {
  const visits = [];
  const writes = [];
  const selections = [
    ['silent', methods], ['error', ['debug', 'info', 'warn']], ['info', ['debug']],
  ];
  for (const [level, selected] of selections) {
    const logger = createLogger({ level, stream: { write: value => writes.push(value) } });
    for (const method of selected) {
      logger[method]('synthetic.suppressed', {
        get detail() { visits.push(`${level}/${method}/getter`); return 'synthetic'; },
      });
      logger[method]('synthetic.suppressed', new Proxy({}, {
        ownKeys() { visits.push(`${level}/${method}/enumerate`); return []; },
      }));
    }
  }
  assert.deepEqual(writes, []);
  assert.deepEqual(visits, []);
});

test('enabled field failures do not escape or prevent subsequent logging', () => {
  const output = [];
  const escaped = [];
  const logger = createLogger({ level: 'debug', stream: { write: value => output.push(value) } });
  const inputs = [
    { get detail() { throw new Error('synthetic getter failed'); } },
    new Proxy({}, { ownKeys() { throw new Error('synthetic enumeration failed'); } }),
  ];
  for (const method of methods) {
    for (const fields of inputs) {
      try { logger[method]('synthetic.failed', fields); }
      catch (error) { escaped.push(error.message); }
    }
    logger[method]('synthetic.recovered', { requestId: method });
  }
  assert.deepEqual(output.map(line => JSON.parse(line).event), methods.map(() => 'synthetic.recovered'));
  assert.deepEqual(output.map(line => JSON.parse(line).level), methods);
  assert.deepEqual(escaped, []);
});

test('enabled getters are evaluated once and preserve bounded log fields', () => {
  const output = [];
  let visits = 0;
  const logger = createLogger({ level: 'error', stream: { write: value => output.push(value) } });
  logger.error('synthetic.visible', {
    get detail() { visits += 1; return { token: 'synthetic-token', text: 'x'.repeat(2001) }; },
    error: new Error('synthetic failure'), level: 'debug', event: 'wrong', time: 'wrong',
  });
  assert.equal(visits, 1);
  assert.equal(output.length, 1);
  assert.ok(output[0].endsWith('\n'));
  const record = JSON.parse(output[0]);
  assert.deepEqual(record.detail, { token: 'synthetic-token', text: 'x'.repeat(2000) });
  assert.deepEqual(record.error, { name: 'Error', message: 'synthetic failure' });
  assert.equal(record.level, 'error');
  assert.equal(record.event, 'synthetic.visible');
  assert.ok(Number.isFinite(Date.parse(record.time)));
});
