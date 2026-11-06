import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/logger.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
const { createLogger } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const levels = ['debug', 'info', 'warn', 'error'];
const cases = [
  { name: '99 fields', count: 99, placement: 'absent', retained: 99 },
  { name: '100 fields', count: 100, placement: 'absent', retained: 100 },
  { name: 'late level field', count: 101, placement: 'last', retained: 100 },
  { name: 'early level field', count: 101, placement: 'first', retained: 99 },
];

for (const fixture of cases) {
  test(`log severity survives the entry limit with ${fixture.name}`, context => {
    const lines = [];
    const logger = createLogger({ level: 'debug', stream: { write: line => lines.push(line) } });
    const entries = Array.from({ length: fixture.count }, (_, index) => [`field_${index}`, index]);
    const fields = Object.fromEntries([
      ...(fixture.placement === 'first' ? [['level', 'spoof']] : []),
      ...entries, ...(fixture.placement === 'last' ? [['level', 'spoof']] : []),
    ]);
    for (const level of levels) logger[level](`synthetic.${level}`, fields);
    const records = lines.map(line => JSON.parse(line));
    context.diagnostic(JSON.stringify({ fixture: fixture.name,
      levels: records.map(record => record.level ?? null),
      retainedFields: records.map(record => Object.keys(record).filter(key => key.startsWith('field_')).length) }));
    assert.equal(records.length, levels.length);
    for (const [index, record] of records.entries()) {
      assert.ok(lines[index].endsWith('\n'));
      assert.equal(record.event, `synthetic.${levels[index]}`);
      assert.ok(Number.isFinite(Date.parse(record.time)));
      const data = Object.fromEntries(Object.entries(record).filter(([key]) => key.startsWith('field_')));
      assert.deepEqual(data, Object.fromEntries(entries.slice(0, fixture.retained)));
    }
    assert.equal(fields.level, fixture.placement === 'absent' ? undefined : 'spoof',
      'logging must not mutate caller fields');
    assert.deepEqual(records.map(record => record.level), levels,
      'the entry budget must not remove severity metadata');
  });
}
