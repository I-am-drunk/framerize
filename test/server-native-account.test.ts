import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { checkNativeAccount, verifyRuntimeFiles } from '../scripts/native-account-smoke.js';
import { accountServerFixture } from '../scripts/fixtures/installed-server.js';

test('native runtime checks distinguish active extraction from completed cleanup', () => {
  verifyRuntimeFiles([], 0);
  verifyRuntimeFiles(['framerize-castle-runtime-a1B2c3'], 1);
  for (const names of [[], ['castle-library-a1B2c3'], ['unknown'],
    ['../framerize-castle-runtime-a1B2c3'], ['framerize-castle-runtime-a/b'],
    ['framerize-castle-runtime-a1B2c3', 'castle-library-a1B2c3']]) {
    assert.throws(() => verifyRuntimeFiles(names, 1));
  }
  assert.throws(() => verifyRuntimeFiles(['framerize-castle-runtime-a1B2c3'], 0));
});

test('native account fixture uses selected references without secrets in arguments', async () => {
  const root = mkdtempSync(join(tmpdir(), 'native-account-policy-'));
  try {
    const config = accountServerFixture(root, {}, 'http://127.0.0.1:1');
    assert.doesNotMatch(config.args.join(' '), /synthetic-account|synthetic-server-credential/);
    assert.equal(config.env.FIXTURE_ACCOUNT_SESSION, 'session=synthetic-account');
    assert.equal(config.args[config.args.indexOf('--account-id') + 1], 'selected');
    const accounts = readFileSync(join(root, 'accounts.json'), 'utf8');
    for (const secret of [config.env.FIXTURE_ACCOUNT_SESSION, config.env.FIXTURE_SERVER_TOKEN]) {
      assert.equal(accounts.includes(secret), false);
      assert.equal(config.args.join(' ').includes(secret), false);
    }
    assert.deepEqual(JSON.parse(accounts), [
      { id: 'unused', projectId: 'unused-project', castleDeviceId: 'synthetic-unused-device',
        session: { env: 'UNSET_ACCOUNT_SESSION' } },
      { id: 'selected', projectId: 'selected-project', castleDeviceId: 'synthetic-selected-device',
        session: { env: 'FIXTURE_ACCOUNT_SESSION' } },
    ]);
    await assert.rejects(checkNativeAccount(root), /regular file/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
