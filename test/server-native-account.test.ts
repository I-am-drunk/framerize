import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { checkNativeAccount } from '../scripts/native-account-smoke.js';
import { accountServerFixture } from '../scripts/fixtures/installed-server.js';

test('native account fixture uses selected references without secrets in arguments', async () => {
  const root = mkdtempSync(join(tmpdir(), 'native-account-policy-'));
  try {
    const config = accountServerFixture(root, {}, 'http://127.0.0.1:1');
    assert.doesNotMatch(config.args.join(' '), /synthetic-account|synthetic-server-credential/);
    assert.equal(config.env.FIXTURE_ACCOUNT_SESSION, 'session=synthetic-account');
    assert.equal(config.args[config.args.indexOf('--account-id') + 1], 'selected');
    const accounts = readFileSync(join(root, 'accounts.json'), 'utf8');
    assert.doesNotMatch(accounts, /session=|synthetic-/);
    assert.deepEqual(JSON.parse(accounts).map((account: { id: string }) => account.id), ['unused', 'selected']);
    await assert.rejects(checkNativeAccount(root), /regular file/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
