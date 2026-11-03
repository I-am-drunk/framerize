import test from 'node:test';
import assert from 'node:assert/strict';
import { parseArgs, run } from '../src/cli.js';
import { parseConfig } from '../src/config.js';

test('default CLI config preserves loopback and default port', () => {
  const config = parseConfig(parseArgs([], {}).values);
  assert.equal(config.host, '127.0.0.1');
  assert.equal(config.port, 8788);
});

test('check validates without listening', async () => {
  assert.equal(await run(['check']), 0);
});

test('unknown options fail', () => {
  assert.throws(() => parseArgs(['serve', '--unknown']), /unknown option/);
});
