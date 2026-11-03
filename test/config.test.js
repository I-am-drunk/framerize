import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { ConfigError, loadAccounts, parseConfig } from '../src/config.js';

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

test('account upstream URLs are normalized and restricted', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-'));
  const file = path.join(directory, 'accounts.json');
  await writeFile(file, JSON.stringify([{ id: 'one', upstream: 'https://example.test/base/' }]));
  assert.equal((await loadAccounts(file))[0].upstream, 'https://example.test/base');
  for (const upstream of ['ftp://example.test', 'not a url', 'https://user:pass@example.test']) {
    await writeFile(file, JSON.stringify([{ id: 'one', upstream }]));
    await assert.rejects(loadAccounts(file), ConfigError);
  }
});

test('account schema rejects invalid and duplicate records', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-'));
  const file = path.join(directory, 'accounts.json');
  for (const account of [null, { id: 'bad id' }, { id: 'one', enabled: 'yes' },
    { id: 'one', model: 1 }, { id: 'one', provider: 'other' },
    { id: 'one', authorizationEnv: 'lower' }, { id: 'one', provider: 'framer' },
    { id: 'one', sessionEnv: 'bad-name' }, { id: 'one', sessionFile: '' }]) {
    await writeFile(file, JSON.stringify([account]));
    await assert.rejects(loadAccounts(file), ConfigError);
  }
  await writeFile(file, JSON.stringify([{ id: 'one' }, { id: 'one' }]));
  await assert.rejects(loadAccounts(file), /duplicate account id/);
});

test('invalid account files and timer ranges fail clearly', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-'));
  const file = path.join(directory, 'accounts.json');
  await writeFile(file, '{}');
  await assert.rejects(loadAccounts(file), /must be an array/);
  await assert.rejects(loadAccounts(path.join(directory, 'missing.json')), /cannot read/);
  assert.throws(() => parseConfig({ upstream: 'ftp://example.test' }), /HTTP or HTTPS/);
  assert.throws(() => parseConfig({ requestTimeoutMs: 2_147_483_648 }), /timer range/);
});
