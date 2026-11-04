import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { checkNativeAccount, verifyRuntimeFiles } from '../scripts/native-account-smoke.js';
import { accountServerFixture, assertAccountFixtureDoesNotLeakSecrets } from '../scripts/fixtures/installed-server.js';

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
    assertAccountFixtureDoesNotLeakSecrets(accounts, config.env, config.args);
    assert.deepEqual(JSON.parse(accounts), [
      { id: 'unused', projectId: 'unused-project', castleDeviceId: 'synthetic-unused-device',
        session: { env: 'UNSET_ACCOUNT_SESSION' } },
      { id: 'selected', projectId: 'selected-project', castleDeviceId: 'synthetic-selected-device',
        session: { env: 'FIXTURE_ACCOUNT_SESSION' } },
    ]);
    await assert.rejects(checkNativeAccount(root), /regular file/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

test('account fixture secret policy rejects embedded values but permits synthetic device identifiers', () => {
  const env = { FIXTURE_ACCOUNT_SESSION: 'session=fixture-secret', FIXTURE_SERVER_TOKEN: 'fixture-server-secret' };
  const accounts = JSON.stringify([
    { id: 'selected', castleDeviceId: 'synthetic-selected-device', session: { env: 'FIXTURE_ACCOUNT_SESSION' } },
  ]);
  assert.doesNotThrow(() => assertAccountFixtureDoesNotLeakSecrets(accounts, env, []));
  assert.throws(() => assertAccountFixtureDoesNotLeakSecrets(`${accounts}session=fixture-secret`, env, []),
    /accounts\.json must not embed/);
  assert.throws(() => assertAccountFixtureDoesNotLeakSecrets(accounts, env, ['--token', 'fixture-server-secret']),
    /fixture CLI args must not embed/);
});
