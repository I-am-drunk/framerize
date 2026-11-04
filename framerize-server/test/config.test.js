import test from 'node:test';
import assert from 'node:assert/strict';
import { loadConfig } from '../dist/config.js';

test('Framer selection requires account metadata and rejects mixed credential paths', () => {
  const base = { backend: 'framer', accountsFile: '/synthetic/accounts.json' };
  assert.equal(loadConfig(base, {}).framerOrigin, 'https://api.framer.com');
  for (const extra of [{ accountsFile: '' }, { backend: 'unknown' }, { accountId: '../account' },
    { upstream: 'https://synthetic.invalid' }, { authorizationEnv: 'TOKEN' },
    { framerOrigin: 'http://example.com' }, { framerOrigin: 'https://example.com/path' }]) {
    assert.throws(() => loadConfig({ ...base, ...extra }, {}), { name: 'ConfigurationError' });
  }
  assert.throws(() => loadConfig({ accountId: 'selected' }, {}), { name: 'ConfigurationError' });
  const selected = loadConfig({}, { FRAMERIZE_SERVER_BACKEND: 'framer',
    FRAMERIZE_SERVER_ACCOUNTS: base.accountsFile, FRAMERIZE_SERVER_ACCOUNT: 'selected',
    FRAMERIZE_SERVER_FRAMER_ORIGIN: 'http://127.0.0.1:1234' });
  assert.equal(selected.accountId, 'selected');
  assert.equal(selected.framerOrigin, 'http://127.0.0.1:1234');
});

test('configuration freezes validated defaults and explicit values', () => {
  const config = loadConfig({ port: 0, serverToken: 'fixture', idleTimeoutMs: '250' }, {});
  assert.equal(config.host, '127.0.0.1');
  assert.equal(config.idleTimeoutMs, 250);
  assert.equal(config.inferenceTimeoutMs, 300_000);
  assert.equal(config.headersTimeoutMs, 15_000);
  assert.equal(loadConfig({ inferenceTimeoutMs: '75000' }, {}).inferenceTimeoutMs, 75000);
  assert.equal(Object.isFrozen(config), true);
  assert.throws(() => { config.port = 1; }, TypeError);
});

test('configuration defaults to no token and supports explicit token environments', () => {
  assert.equal(loadConfig({}, {}).serverToken, '');
  const config = loadConfig({ serverTokenEnv: 'FIXTURE_SERVER_TOKEN' },
    { FIXTURE_SERVER_TOKEN: 'from-environment' });
  assert.equal(config.serverToken, 'from-environment');
  assert.equal(loadConfig({ serverTokenEnv: 'MISSING_FIXTURE_TOKEN' }, {}).serverToken, '');
});

test('configuration rejects the removed total request deadline', () => {
  assert.throws(() => loadConfig({ serverToken: 'fixture', requestTimeoutMs: 1 }, {}),
    /requestTimeoutMs is not supported/);
});

test('configuration validates integer ranges for ports, bodies and timers', () => {
  for (const [key, value] of [
    ['port', -1], ['port', 65_536], ['maxBodyBytes', 0],
    ['idleTimeoutMs', 0], ['headersTimeoutMs', 2_147_483_648],
    ['inferenceTimeoutMs', 0], ['inferenceTimeoutMs', 1.5], ['inferenceTimeoutMs', 2_147_483_648],
    ['requestHeadersTimeoutMs', 1.5], ['shutdownTimeoutMs', Infinity],
  ]) {
    assert.throws(() => loadConfig({ serverToken: 'fixture', [key]: value }, {}),
      { name: 'ConfigurationError' }, `${key}=${value}`);
  }
});

test('configuration rejects invalid hosts and environment names', () => {
  for (const values of [
    { serverToken: 'fixture', host: '' },
    { serverToken: 'fixture', host: 42 },
    { serverTokenEnv: 'lower-case' },
    { serverToken: 42 },
  ]) {
    assert.throws(() => loadConfig(values, {}), { name: 'ConfigurationError' });
  }
});
