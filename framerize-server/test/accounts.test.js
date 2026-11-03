import test from 'node:test';
import assert from 'node:assert/strict';
import { chmod, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { loadAccounts, loadFramerAccount, readAccountSession } from '../dist/accounts.js';

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

test('JSON stores only frozen credential references and resolves files beside the accounts file', async context => {
  const file = await accountFile(context, [
    { id: 'env', projectId: 'project-a', session: { env: 'SYNTHETIC_SESSION' } },
    { id: 'file', projectId: 'project-b', session: { file: './private-session' } },
  ]);
  const accounts = await loadAccounts(file);
  assert.deepEqual(accounts[0].session, { env: 'SYNTHETIC_SESSION' });
  assert.equal(accounts[1].session.file, path.join(path.dirname(file), 'private-session'));
  assert.equal(Object.isFrozen(accounts[0].session), true);
  assert.equal(Object.isFrozen(accounts[1].session), true);
  assert.equal((await loadFramerAccount(file, 'file')).id, 'file');
  await assert.rejects(loadFramerAccount(file), { message: 'Select one configured Framer account' });
});

for (const session of [null, [], {}, { env: '' }, { env: 4 }, { file: '' }, { file: '../bad\0path' },
  { env: 'VALID', file: 'private-session' }, { cookie: 'synthetic-private' }, { env: 'VALID', token: 'synthetic-private' }]) {
  test('invalid session references never turn JSON into a credential store', async context => {
    const file = await accountFile(context, [{ id: 'fixture', projectId: 'project', session }]);
    await assert.rejects(loadAccounts(file), { message: 'Invalid account session reference' });
  });
}

test('only one enabled configured account may be chosen implicitly', async context => {
  const entry = { id: 'enabled', projectId: 'project', session: { env: 'SYNTHETIC_SESSION' } };
  const file = await accountFile(context, [entry, { ...entry, id: 'disabled', enabled: false }]);
  assert.equal((await loadFramerAccount(file)).id, 'enabled');
  for (const id of ['disabled', 'missing']) {
    await assert.rejects(loadFramerAccount(file, id), { message: 'Select one configured Framer account' });
  }
  await assert.rejects(loadFramerAccount(file, 'bad/id'), { message: 'Invalid account selector' });
  await assert.rejects(loadFramerAccount(await accountFile(context, [{ id: 'metadata-only' }])),
    { message: 'Select one configured Framer account' });
  await assert.rejects(loadAccounts(await accountFile(context, [{ ...entry, projectId: 'bad/project' }])),
    { message: 'Invalid account project' });
});

test('session references read only the chosen environment variable', async () => {
  let unused = 0;
  const env = { SELECTED: 'session=synthetic', get UNUSED() { unused += 1; throw new Error(); } };
  assert.equal(await readAccountSession({ env: 'SELECTED' }, env), 'session=synthetic');
  assert.equal(unused, 0);
  for (const value of [undefined, '', ' secret', 'secret ', 'secret\n', 'secret\r\ninjected:x', '\ufeffsecret', 'x'.repeat(16385)]) {
    await assert.rejects(readAccountSession({ env: 'SELECTED' }, { SELECTED: value }),
      { message: 'Account session unavailable' });
  }
  assert.equal((await readAccountSession({ env: 'SELECTED' }, { SELECTED: 'x'.repeat(16384) })).length, 16384);
  const throwing = { get SELECTED() { throw new Error('synthetic-private'); } };
  await assert.rejects(readAccountSession({ env: 'SELECTED' }, throwing), { message: 'Account session unavailable' });
  await assert.rejects(readAccountSession(new Proxy({}, {
    ownKeys() { throw new Error('synthetic-private'); },
  })), { message: 'Account session unavailable' });
  await assert.rejects(readAccountSession({ env: 'UNUSED' }, env, AbortSignal.abort()),
    { message: 'Account session unavailable' });
  assert.equal(unused, 0);
});

test('private session files accept one final newline and reject invalid or oversized bytes', async context => {
  const manifest = await accountFile(context, []);
  const file = path.join(path.dirname(manifest), 'session');
  for (const ending of ['', '\n', '\r\n']) {
    await writeFile(file, 'session=synthetic' + ending, { mode: 0o600 });
    assert.equal(await readAccountSession({ file }), 'session=synthetic');
  }
  for (const bytes of [Buffer.from([0xff]), Buffer.from('\ufeffsession=synthetic'),
    Buffer.from('secret\n\n'), Buffer.from('secret\r\ninjected:x'), Buffer.alloc(16385, 120)]) {
    await writeFile(file, bytes);
    await assert.rejects(readAccountSession({ file }), { message: 'Account session unavailable' });
  }
  await assert.rejects(readAccountSession({ file: path.dirname(manifest) }),
    { message: 'Account session unavailable' });
  await assert.rejects(readAccountSession({ file: 'relative-secret' }), { message: 'Account session unavailable' });
});

test('session files reject group and other permissions', { skip: process.platform === 'win32' }, async context => {
  const manifest = await accountFile(context, []);
  const file = path.join(path.dirname(manifest), 'session');
  await writeFile(file, 'session=synthetic', { mode: 0o600 });
  await chmod(file, 0o640);
  await assert.rejects(readAccountSession({ file }), { message: 'Account session unavailable' });
  await chmod(file, 0o600);
  assert.equal(await readAccountSession({ file }), 'session=synthetic');
});
