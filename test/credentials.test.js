import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { loadAccounts, readCredential } from '../src/config.js';

test('private credential files are read without JSON secrets', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-credentials-'));
  const credential = path.join(directory, 'session.private');
  await writeFile(credential, 'session=fixture-secret\n');
  assert.equal(await readCredential(credential), 'session=fixture-secret');
  const accountsFile = path.join(directory, 'accounts.json');
  await writeFile(accountsFile, JSON.stringify([{ id: 'one', provider: 'framer',
    projectId: 'project', sessionFile: credential, castleFile: credential }]));
  const [account] = await loadAccounts(accountsFile);
  assert.equal(account.sessionFile, credential);
  assert.equal(account.castleFile, credential);
  assert.equal(JSON.stringify(account).includes('fixture-secret'), false);
});

test('invalid credential references fail safely', async () => {
  const directory = await mkdtemp(path.join(tmpdir(), 'framer-next-credentials-'));
  const nested = path.join(directory, 'nested');
  await mkdir(nested);
  await assert.rejects(readCredential(nested), /small regular file/);
  const oversized = path.join(directory, 'oversized');
  await writeFile(oversized, 'x'.repeat(16 * 1024 + 1));
  await assert.rejects(readCredential(oversized), /small regular file/);
  await assert.rejects(readCredential(path.join(directory, 'missing')));
});
