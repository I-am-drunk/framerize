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

for (const kind of ['object', 'error']) {
  for (const overflow of ['returns', 'throws']) {
    test(`${kind} logging does not read a ${overflow} getter beyond its entry limit`, () => {
      const payload = kind === 'error' ? new Error('synthetic failure') : {};
      const fieldLimit = kind === 'error' ? 98 : 100;
      for (let index = 0; index < fieldLimit; index += 1) payload[`field${index}`] = index;
      let visits = 0;
      Object.defineProperty(payload, 'overflow', { enumerable: true, get() {
        visits += 1;
        if (overflow === 'throws') throw new Error('synthetic overflow');
        return 'not included';
      } });
      const output = [];
      const logger = createLogger({ stream: { write: value => output.push(value) } });
      logger.info('synthetic.boundary', { payload });
      assert.equal(visits, 0, 'the omitted getter must not run');
      assert.equal(output.length, 1, 'the in-range record must still be written');
      const record = JSON.parse(output[0]);
      assert.equal(record.event, 'synthetic.boundary');
      assert.equal(record.level, 'info');
      assert.equal(Object.keys(record.payload).length, 100);
      assert.equal(Object.hasOwn(record.payload, 'overflow'), false);
      for (let index = 0; index < fieldLimit; index += 1) {
        assert.equal(record.payload[`field${index}`], index);
      }
      if (kind === 'error') {
        assert.equal(record.payload.name, 'Error');
        assert.equal(record.payload.message, 'synthetic failure');
      }
    });
  }

  test(`${kind} logging reads and preserves its last included getter once`, () => {
    const payload = kind === 'error' ? new Error('synthetic failure') : {};
    const fieldLimit = kind === 'error' ? 98 : 100;
    for (let index = 0; index < fieldLimit - 1; index += 1) payload[`field${index}`] = index;
    let visits = 0;
    Object.defineProperty(payload, 'last', { enumerable: true, get() {
      visits += 1;
      return { text: 'x'.repeat(2001), parent: payload };
    } });
    const output = [];
    createLogger({ stream: { write: value => output.push(value) } }).warn('synthetic.last', { payload });
    assert.equal(visits, 1);
    const record = JSON.parse(output[0]);
    assert.equal(Object.keys(record.payload).length, 100);
    assert.deepEqual(record.payload.last, { text: 'x'.repeat(2000), parent: '[circular]' });
  });
}

test('enumerable Error metadata retains its existing order and cap accounting', () => {
  const payload = new Error('synthetic failure');
  const visits = { name: 0, message: 0 };
  for (const key of ['name', 'message']) {
    Object.defineProperty(payload, key, { enumerable: true, get() {
      visits[key] += 1;
      return `${key}-${visits[key]}`;
    } });
  }
  for (let index = 0; index < 100; index += 1) payload[`field${index}`] = index;
  const output = [];
  createLogger({ stream: { write: value => output.push(value) } }).error('synthetic.metadata', { payload });
  const record = JSON.parse(output[0]);
  assert.deepEqual(visits, { name: 2, message: 2 });
  assert.equal(record.payload.name, 'name-2');
  assert.equal(record.payload.message, 'message-2');
  assert.equal(Object.keys(record.payload).length, 98);
  assert.equal(record.payload.field95, 95);
  assert.equal(Object.hasOwn(record.payload, 'field96'), false);
});
