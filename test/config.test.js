import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { loadAccounts, parseConfig } from '../src/config.js';

test('config defaults to loopback and validates ports', () => {
  const config = parseConfig({ port: '9000' });
  assert.equal(config.host, '127.0.0.1');
  assert.equal(config.port, 9000);
  assert.throws(() => parseConfig({ port: -1 }), /positive integer/);
  assert.throws(() => parseConfig({ port: 70000 }), /at most/);
});

test('accounts JSON validates ids and rejects embedded secrets', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-'));
  const file = path.join(directory, 'accounts.json');
  await writeFile(file, JSON.stringify([{ id: 'one', enabled: true }, { id: 'two', enabled: false }]));
  assert.deepEqual(await loadAccounts(file), [{ id: 'one', enabled: true }]);
  await writeFile(file, JSON.stringify([{ id: 'one', token: 'secret' }]));
  await assert.rejects(loadAccounts(file), /unknown field: token/);
});
