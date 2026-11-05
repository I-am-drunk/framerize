import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { loadAccounts, loadFramerAccount, readAccountSession } from '../dist/accounts.js';

async function accountFile(context, accounts) {
  const directory = await mkdtemp(join(tmpdir(), 'server-json-accounts-'));
  context.after(() => rm(directory, { recursive: true, force: true }));
  const file = join(directory, 'accounts.json');
  await writeFile(file, JSON.stringify(accounts));
  return file;
}

test('a JSON account can supply its session without a separate file or environment', async context => {
  const session = 'session=synthetic-inline';
  const file = await accountFile(context, [
    { id: 'disabled', enabled: false, projectId: 'other', session: 'unused' },
    { id: 'personal', projectId: 'project', session },
  ]);
  const selected = await loadFramerAccount(file);
  assert.equal(selected.id, 'personal');
  assert.equal(selected.session, session);
  const unreadable = new Proxy({}, { get() { throw new Error('environment must not be read'); } });
  assert.equal(await readAccountSession(selected.session, unreadable), session);
  assert.equal((await loadAccounts(file))[1].session, session);
});

test('inline sessions retain the same value validation as resolved references', async context => {
  for (const session of ['', ' padded', 'trailing ', 'line\nbreak', '\ufeffsession', 'x'.repeat(16_385)]) {
    const file = await accountFile(context, [{ id: 'account', projectId: 'project', session }]);
    await assert.rejects(loadAccounts(file), { message: 'Invalid account session reference' });
    await assert.rejects(readAccountSession(session), { message: 'Account session unavailable' });
  }
  const session = 'x'.repeat(16_384);
  const file = await accountFile(context, [{ id: 'account', projectId: 'project', session }]);
  assert.equal((await loadFramerAccount(file)).session, session);
});

test('inline sessions honor cancellation before returning a value', async () => {
  await assert.rejects(readAccountSession('session=synthetic', {}, AbortSignal.abort()),
    { message: 'Account session unavailable' });
});
