import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { loadConfig } from '../dist/config.js';
import { captureSettings, configuredSettings } from '../dist/settings.js';

async function fixture(t, config, env = {}) {
  const home = await mkdtemp(join(tmpdir(), 'server-settings-test-'));
  t.after(() => rm(home, { recursive: true, force: true }));
  const path = join(home, 'server.json');
  await writeFile(path, JSON.stringify(captureSettings(loadConfig(config, env), env)));
  return path;
}

test('saved settings preserve named credentials and resolve file references for a service', async t => {
  const env = { SERVER_KEY: 'synthetic_server', UPSTREAM_KEY: 'synthetic_upstream', UNUSED_KEY: 'not-captured' };
  const path = await fixture(t, { serverTokenEnv: 'SERVER_KEY', authorizationEnv: 'UPSTREAM_KEY',
    accountsFile: 'synthetic-accounts.json', upstream: 'http://127.0.0.1:1234' }, env);
  const loaded = await configuredSettings(path, {}, {});
  assert.equal(loaded.config.serverToken, env.SERVER_KEY);
  assert.equal(loaded.config.accountsFile, resolve('synthetic-accounts.json'));
  assert.equal(loaded.env.UPSTREAM_KEY, env.UPSTREAM_KEY);
  assert.equal(loaded.env.UNUSED_KEY, undefined);
});

test('flags override env and saved sources; disabling LAN restores loopback', async t => {
  const path = await fixture(t, { privateNetworkOnly: true, port: 1234, serverTokenFile: '/old/token' });
  const env = { FRAMERIZE_SERVER_PORT: '2345', FRAMERIZE_SERVER_LAN: 'false',
    FRAMERIZE_SERVER_TOKEN_FILE: '/env/token', NEW_KEY: 'new_synthetic_key' };
  const loaded = await configuredSettings(path, { port: '3456', serverTokenEnv: 'NEW_KEY' }, env);
  assert.equal(loaded.config.port, 3456);
  assert.equal(loaded.config.host, '127.0.0.1');
  assert.equal(loaded.config.privateNetworkOnly, false);
  assert.equal(loaded.config.serverTokenFile, '');
  assert.equal(loaded.config.serverToken, env.NEW_KEY);
  assert.equal((await configuredSettings(path, {}, env)).config.port, 2345);
  await assert.rejects(configuredSettings(path, { serverTokenEnv: 'MISSING' }, env), /server token/);
});

test('invalid saved JSON fails without modifying the settings file', async t => {
  const path = await fixture(t, {});
  await writeFile(path, '{not-json');
  await assert.rejects(configuredSettings(path, {}, {}), /settings/);
  assert.equal(await readFile(path, 'utf8'), '{not-json');
});

test('tokenless settings stay tokenless after restart and accept an explicit env token later', async t => {
  const path = await fixture(t, {});
  assert.equal((await configuredSettings(path, {}, {})).config.serverToken, '');
  const changed = await configuredSettings(path, {}, { FRAMERIZE_SERVER_TOKEN: 'later_token' });
  assert.equal(changed.config.serverToken, 'later_token');
});
