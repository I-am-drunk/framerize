import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { loadAccounts } from '../dist/accounts.js';

async function accountFile(t, value) {
  const directory = await mkdtemp(path.join(tmpdir(), 'framerize-accounts-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const file = path.join(directory, 'accounts.json');
  await writeFile(file, JSON.stringify(value));
  return file;
}

test('account metadata is validated, frozen and credential-free', async (t) => {
  const file = await accountFile(t, [{
    id: 'fixture', label: 'Fixture account', enabled: true,
    model: 'default-model', models: ['default-model', 'other-model'],
  }]);
  const accounts = await loadAccounts(file);
  assert.deepEqual(accounts, [{
    id: 'fixture', label: 'Fixture account', enabled: true,
    model: 'default-model', models: ['default-model', 'other-model'],
  }]);
  assert.equal(Object.isFrozen(accounts), true);
  assert.equal(Object.isFrozen(accounts[0]), true);
  assert.equal(Object.isFrozen(accounts[0].models), true);
  assert.equal('authorization' in accounts[0], false);
});

test('account metadata rejects credentials, unknown fields and duplicate IDs', async (t) => {
  for (const accounts of [
    [{ id: 'fixture', authorization: 'secret' }],
    [{ id: 'fixture', cookie: 'secret' }],
    [{ id: 'duplicate' }, { id: 'duplicate' }],
  ]) {
    await assert.rejects(loadAccounts(await accountFile(t, accounts)),
      { name: 'ConfigurationError' });
  }
});

test('account metadata rejects malformed JSON shapes without exposing values', async (context) => {
  const secret = 'synthetic-account-secret';
  for (const value of [null, {}, 'text', [null], [[]], [false], [{ id: '' }],
    [{ id: secret, label: 1 }], [{ id: secret, enabled: 'true' }],
    [{ id: secret, model: 'x'.repeat(257) }], [{ id: secret, models: [secret, 1] }],
    [{ id: secret, models: [''] }], [{ id: secret, models: 'model' }],
    [{ id: secret, models: Array(101).fill('model') }]]) {
    await assert.rejects(loadAccounts(await accountFile(context, value)), (error) => {
      assert.equal(error.name, 'ConfigurationError');
      assert.doesNotMatch(error.message, new RegExp(secret));
      return true;
    });
  }
});

test('account metadata handles omitted input and valid boundary values', async (context) => {
  const empty = await loadAccounts();
  assert.deepEqual(empty, []);
  assert.equal(Object.isFrozen(empty), true);
  const values = [{ id: 'x'.repeat(64), enabled: false, label: '', model: 'x'.repeat(256),
    models: Array(100).fill('x'.repeat(256)) }];
  assert.deepEqual(await loadAccounts(await accountFile(context, values)), values);
});

test('account file limits and malformed bytes fail with fixed diagnostics', async (context) => {
  const file = await accountFile(context, []);
  for (const bytes of [Buffer.from('{'), Buffer.from([0xff]), Buffer.alloc(1024 * 1024 + 1, 32)]) {
    await writeFile(file, bytes);
    await assert.rejects(loadAccounts(file),
      { name: 'ConfigurationError', message: 'Account metadata unavailable' });
  }
});
