import assert from 'node:assert/strict';
import test from 'node:test';
import { loadConfig, readServerToken } from '../dist/config.js';

test('explicit token variables must resolve while the default remains optional', () => {
  assert.equal(loadConfig({}, {}).serverToken, '');
  assert.equal(loadConfig({ serverTokenEnv: '' }, {}).serverToken, '');
  for (const token of [undefined, '']) {
    const env = { SYNTHETIC_TOKEN: token };
    assert.throws(() => loadConfig({ serverTokenEnv: 'SYNTHETIC_TOKEN' }, env),
      { name: 'ConfigurationError', message: 'Invalid server token' });
    assert.throws(() => loadConfig({}, { ...env, FRAMERIZE_SERVER_TOKEN_ENV: 'SYNTHETIC_TOKEN' }),
      { name: 'ConfigurationError', message: 'Invalid server token' });
  }
});

test('explicit token sources override environment-selected alternatives', () => {
  const env = { FRAMERIZE_SERVER_TOKEN_FILE: '/unused/token', CHOSEN: 'synthetic-chosen' };
  const named = loadConfig({ serverTokenEnv: 'CHOSEN' }, env);
  assert.equal(named.serverToken, 'synthetic-chosen');
  assert.equal(named.serverTokenFile, '');
  const direct = loadConfig({ serverToken: 'synthetic-direct' }, env);
  assert.equal(direct.serverToken, 'synthetic-direct');
  assert.equal(direct.serverTokenFile, '');
  const file = loadConfig({ serverTokenFile: '/chosen/token' }, { FRAMERIZE_SERVER_TOKEN_ENV: 'MISSING' });
  assert.equal(file.serverTokenFile, '/chosen/token');
  assert.equal(file.serverToken, '');
});

test('resolved token configurations can be validated again', () => {
  for (const values of [{}, { serverToken: 'synthetic-inline' },
    { serverTokenEnv: 'SYNTHETIC_TOKEN' }, { serverTokenFile: '/chosen/token' }]) {
    const first = loadConfig(values, { SYNTHETIC_TOKEN: 'synthetic-named' });
    const second = loadConfig(first, { FRAMERIZE_SERVER_TOKEN: 'unused' });
    assert.equal(second.serverToken, first.serverToken);
    assert.equal(second.serverTokenFile, first.serverTokenFile);
  }
});

test('equally explicit token sources are rejected', () => {
  const both = { serverTokenEnv: 'SYNTHETIC_TOKEN', serverTokenFile: '/unused/token' };
  assert.throws(() => loadConfig(both, { SYNTHETIC_TOKEN: 'synthetic-token' }),
    { name: 'ConfigurationError', message: 'Invalid server credential references' });
  assert.throws(() => loadConfig({}, {
    FRAMERIZE_SERVER_TOKEN_ENV: both.serverTokenEnv,
    FRAMERIZE_SERVER_TOKEN_FILE: both.serverTokenFile, SYNTHETIC_TOKEN: 'synthetic-token',
  }), { name: 'ConfigurationError', message: 'Invalid server credential references' });
});

test('agents can configure every Server limit through environment variables', () => {
  const options = [
    ['maxBodyBytes', 'FRAMERIZE_SERVER_MAX_BODY_BYTES', 2048],
    ['headersTimeoutMs', 'FRAMERIZE_SERVER_HEADERS_TIMEOUT_MS', 1000],
    ['inferenceTimeoutMs', 'FRAMERIZE_SERVER_INFERENCE_TIMEOUT_MS', 2000],
    ['idleTimeoutMs', 'FRAMERIZE_SERVER_IDLE_TIMEOUT_MS', 3000],
    ['requestHeadersTimeoutMs', 'FRAMERIZE_SERVER_REQUEST_HEADERS_TIMEOUT_MS', 4000],
    ['shutdownTimeoutMs', 'FRAMERIZE_SERVER_SHUTDOWN_TIMEOUT_MS', 500],
  ];
  for (const [name, variable, value] of options) {
    assert.equal(loadConfig({}, { [variable]: String(value) })[name], value);
    assert.throws(() => loadConfig({}, { [variable]: 'invalid' }),
      { message: `Invalid ${variable}` });
    assert.equal(loadConfig({ [name]: value }, { [variable]: 'invalid' })[name], value);
  }
});

test('agents can select log verbosity without a flag', () => {
  assert.equal(loadConfig({}, { FRAMERIZE_SERVER_LOG_LEVEL: 'debug' }).logLevel, 'debug');
  assert.equal(loadConfig({ logLevel: 'silent' }, { FRAMERIZE_SERVER_LOG_LEVEL: 'debug' }).logLevel, 'silent');
  assert.throws(() => loadConfig({}, { FRAMERIZE_SERVER_LOG_LEVEL: 'invalid' }));
});

test('agents can select optional credential sources through the environment', async () => {
  const config = loadConfig({}, {
    FRAMERIZE_SERVER_TOKEN_ENV: 'SYNTHETIC_API_TOKEN',
    SYNTHETIC_API_TOKEN: 'synthetic-api-token',
    FRAMERIZE_SERVER_AUTHORIZATION_ENV: 'SYNTHETIC_UPSTREAM',
  });
  assert.equal(await readServerToken(config), 'synthetic-api-token');
  assert.equal(config.authorizationEnv, 'SYNTHETIC_UPSTREAM');
  const fileConfig = loadConfig({}, {
    FRAMERIZE_SERVER_TOKEN_FILE: '/synthetic/api-token',
    FRAMERIZE_SERVER_AUTHORIZATION_FILE: '/synthetic/upstream-token',
  });
  assert.equal(fileConfig.serverTokenFile, '/synthetic/api-token');
  assert.equal(fileConfig.authorizationFile, '/synthetic/upstream-token');
  assert.equal(loadConfig({ serverTokenFile: '' }, {
    FRAMERIZE_SERVER_TOKEN_FILE: '/synthetic/api-token',
  }).serverTokenFile, '');
  assert.throws(() => loadConfig({}, {
    FRAMERIZE_SERVER_AUTHORIZATION_ENV: 'SYNTHETIC_UPSTREAM',
    FRAMERIZE_SERVER_AUTHORIZATION_FILE: '/synthetic/upstream-token',
  }));
});
