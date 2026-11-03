import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { loadAccounts } from '../src/accounts.js';

async function accountFile(t, value) {
  const directory = await mkdtemp(path.join(tmpdir(), 'framerize-accounts-'));
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
  assert.equal('authorization' in accounts[0], false);
});
