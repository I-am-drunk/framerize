import test from 'node:test';
import assert from 'node:assert/strict';
import { loadConfig } from '../dist/config.js';
import { run } from '../dist/cli.js';

const ENV = { FRAMERIZE_SERVER_HOST: '0.0.0.0', FRAMERIZE_SERVER_PORT: '9000',
  FRAMERIZE_SERVER_UPSTREAM: 'https://inference.example.test/api' };

test('environment selection keeps defaults when no variables are set', () => {
  const config = loadConfig({}, {});
  assert.equal(config.host, '127.0.0.1');
  assert.equal(config.port, 8789);
  assert.equal(config.upstream, '');
});

test('environment supplies host, port and upstream through existing validation', () => {
  const config = loadConfig({}, ENV);
  assert.equal(config.host, '0.0.0.0');
  assert.equal(config.port, 9000);
  assert.equal(config.upstream, 'https://inference.example.test/api');
  assert.equal(loadConfig({}, { FRAMERIZE_SERVER_PORT: '0' }).port, 0);
});

test('explicit values override environment values, including invalid ones', () => {
  const config = loadConfig({ host: '::1', port: 0, upstream: '' }, ENV);
  assert.equal(config.host, '::1');
  assert.equal(config.port, 0);
  assert.equal(config.upstream, '');
  const invalid = { FRAMERIZE_SERVER_HOST: '', FRAMERIZE_SERVER_PORT: 'x', FRAMERIZE_SERVER_UPSTREAM: 'ftp://x' };
  assert.equal(loadConfig({ host: 'localhost', port: '1', upstream: 'http://a.test/' }, invalid).port, 1);
});

test('invalid selected environment values fail with the variable name', () => {
  for (const [name, value] of [
    ['FRAMERIZE_SERVER_HOST', ''], ['FRAMERIZE_SERVER_HOST', 'a b'], ['FRAMERIZE_SERVER_HOST', 'a/b'],
    ['FRAMERIZE_SERVER_PORT', ''], ['FRAMERIZE_SERVER_PORT', '-1'], ['FRAMERIZE_SERVER_PORT', '65536'],
    ['FRAMERIZE_SERVER_PORT', '1.5'], ['FRAMERIZE_SERVER_PORT', '0x10'],
    ['FRAMERIZE_SERVER_UPSTREAM', 'not a url'], ['FRAMERIZE_SERVER_UPSTREAM', 'ftp://a.test/'],
  ]) {
    assert.throws(() => loadConfig({}, { [name]: value }),
      { name: 'ConfigurationError', message: `Invalid ${name}` }, `${name}=${value}`);
  }
  assert.throws(() => loadConfig({ port: 'x' }, ENV), { message: 'Invalid port' });
});

test('environment errors never include raw secrets from the selected value', () => {
  for (const upstream of ['https://user:raw-secret@a.test/', 'https://a.test/?token=raw-secret',
    'https://a.test/#raw-secret']) {
    assert.throws(() => loadConfig({}, { FRAMERIZE_SERVER_UPSTREAM: upstream }), (error) => {
      assert.equal(error.message, 'Invalid FRAMERIZE_SERVER_UPSTREAM');
      assert.doesNotMatch(`${error.message}\n${error.stack}`, /raw-secret/);
      return true;
    });
  }
  const config = loadConfig({}, { ...ENV, FRAMERIZE_SERVER_TOKEN: 'server-secret' });
  assert.equal(config.serverToken, 'server-secret');
  assert.doesNotMatch(JSON.stringify(config), /server-secret/);
});

test('CLI options override environment values without reading process.env', async () => {
  const seen = [];
  const dependencies = {
    env: ENV, logger: { info() {} },
    createProvider: (config) => ({ config, async close() {} }),
    createServer: async (config, { provider }) => {
      seen.push({ config, provider });
      return { async close() {} };
    },
  };
  assert.equal(await run(['check', '--port', '0', '--host', 'localhost'], dependencies), 0);
  assert.equal(seen[0].config.host, 'localhost');
  assert.equal(seen[0].config.port, 0);
  assert.equal(seen[0].config.upstream, ENV.FRAMERIZE_SERVER_UPSTREAM);
  assert.equal(seen[0].provider.config.serverTokenEnv, 'FRAMERIZE_SERVER_TOKEN');
  assert.deepEqual({ ...seen[0].provider.config, serverTokenEnv: '' }, seen[0].config);
  assert.equal(seen[0].provider.config.serverToken, seen[0].config.serverToken);
  await assert.rejects(run(['check'], { ...dependencies, env: { FRAMERIZE_SERVER_PORT: 'x' } }),
    { message: 'Invalid FRAMERIZE_SERVER_PORT' });
});
