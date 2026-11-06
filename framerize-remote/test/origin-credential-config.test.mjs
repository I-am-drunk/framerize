import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const root = process.env.FRAMERIZE_REMOTE_SOURCE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
const require = createRequire(join(root, 'package.json'));
const { build } = require('esbuild');
const bundled = await build({ entryPoints: [join(root, 'src/configStore.ts')],
  bundle: true, write: false, platform: 'node', format: 'esm', target: 'node22.12' });
const { loadRunConfig, sameRemoteOrigin } = await import(
  `data:text/javascript;base64,${Buffer.from(bundled.outputFiles[0].text).toString('base64')}`);

test('origin comparison preserves canonical equivalence without DNS', () => {
  for (const [first, second, expected] of [
    ['https://EXAMPLE.invalid:443/', 'https://example.invalid', true],
    ['http://127.1:80', 'http://127.0.0.1/', true],
    ['http://[0:0:0:0:0:0:0:1]', 'http://[::1]:80/', true],
    ['https://example.invalid', 'http://example.invalid', false],
    ['http://127.0.0.1:3000', 'http://127.0.0.1:3001', false],
    ['https://first.invalid', 'https://second.invalid', false],
    ['https://example.invalid/path', 'https://example.invalid', false],
    ['not an origin', 'https://example.invalid', false],
  ]) assert.equal(sameRemoteOrigin(first, second), expected);
});

test('all saved reference types remain bound to their canonical origin', async context => {
  const directory = await mkdtemp(join(tmpdir(), 'frm-origin-config-'));
  context.after(() => rm(directory, { recursive: true, force: true }));
  const profile = join(directory, 'profile.json');
  const localToken = { kind: 'value', value: 'synthetic-local-only' };
  for (const serverToken of [
    { kind: 'value', value: 'synthetic-origin-only' },
    { kind: 'env', name: 'SYNTHETIC_ORIGIN_TOKEN' },
    { kind: 'file', path: 'synthetic-token.txt' },
  ]) {
    await writeFile(profile, JSON.stringify({ remote: 'https://EXAMPLE.invalid:443/',
      serverToken, localToken }), { mode: 0o600 });
    const same = await loadRunConfig(profile, { FRM_REMOTE: 'https://example.invalid' });
    assert.deepEqual(same.serverToken, serverToken.kind === 'file'
      ? { kind: 'file', path: join(directory, serverToken.path) } : serverToken);
    const changed = await loadRunConfig(profile, { FRM_REMOTE: 'https://other.invalid' });
    assert.equal(changed.serverToken, undefined);
    assert.deepEqual(changed.localToken, localToken);
    const explicit = await loadRunConfig(profile, { FRM_REMOTE: 'https://other.invalid',
      FRM_SERVER_TOKEN: 'synthetic-explicit-token' });
    assert.deepEqual(explicit.serverToken, { kind: 'env', name: 'FRM_SERVER_TOKEN' });
  }
});
