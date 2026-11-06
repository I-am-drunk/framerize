import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import http from 'node:http';
import https from 'node:https';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { build } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const built = await build({ absWorkingDir: root, bundle: true, write: false,
  platform: 'node', format: 'esm', stdin: { resolveDir: root, contents:
    "export { resolveInstance } from './framerize-remote/src/resolveInstance.ts'; export { frmSettings } from './framerize-remote/src/frmEnv.ts';" },
  plugins: [{ name: 'core-source', setup(builder) {
    builder.onResolve({ filter: /^@framerize\/core$/ }, () => ({ path: `${root}framerize-core/src/index.ts` }));
  } }],
});
const bytes = built.outputFiles[0].contents;
const { resolveInstance, frmSettings } = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
console.log(JSON.stringify({ runtime: process.version,
  bundle: createHash('sha256').update(bytes).digest('hex'), environment: 'synthetic case-insensitive lookup' }));

function windowsEnvironment(values) {
  return new Proxy(values, { get(target, property) {
    if (typeof property !== 'string') return Reflect.get(target, property);
    const key = Object.keys(target).find(name => name.toLowerCase() === property.toLowerCase());
    return target[key ?? property];
  } });
}

async function fixture(context, values, saved) {
  const directory = await mkdtemp(join(tmpdir(), 'remote-env-lookup-'));
  const path = join(directory, 'profile.json');
  let instance;
  let networkCalls = 0;
  const env = windowsEnvironment(values);
  const before = JSON.stringify(values);
  context.after(async () => {
    try {
      await instance?.backend.close();
      assert.equal(JSON.stringify(values), before);
      assert.equal(networkCalls, 0);
      if (saved) assert.equal(await readFile(path, 'utf8'), saved);
      else await assert.rejects(stat(path), { code: 'ENOENT' });
    } finally { await rm(directory, { recursive: true, force: true }); }
  });
  if (saved) await writeFile(path, saved, { mode: 0o600 });
  for (const transport of [http, https]) context.mock.method(transport, 'request', () => {
    networkCalls += 1;
    throw new Error('Network is forbidden in this configuration test');
  });
  return { env, async resolve(overrides) {
    try { instance = await resolveInstance(path, env, overrides); return { instance }; }
    catch (error) { return { error: { code: error.code, message: error.message } }; }
  } };
}

test('environment-only config preserves case-insensitive variable lookup', { timeout: 3000 }, async context => {
  const state = await fixture(context, { FrM_Remote: 'http://127.0.0.1:43210',
    FrM_Host: '127.0.0.2', FrM_Port: '43211' });
  assert.equal(frmSettings(state.env).remote, 'http://127.0.0.1:43210');
  const result = await state.resolve();
  assert.equal(result.error, undefined);
  assert.equal(result.instance.config.remote, 'http://127.0.0.1:43210');
  assert.equal(result.instance.config.host, '127.0.0.2');
  assert.equal(result.instance.config.port, 43211);
});

test('saved config keeps token references supplied through mixed-case variables', { timeout: 3000 }, async context => {
  const saved = '{"remote":"http://127.0.0.1:43210","port":0}\n';
  const state = await fixture(context, { FrM_Server_Token: 'synthetic-server-token',
    FrM_Local_Token: 'synthetic-local-token' }, saved);
  assert.equal(frmSettings(state.env).localToken.name, 'FRM_LOCAL_TOKEN');
  const result = await state.resolve();
  assert.equal(result.error, undefined);
  assert.deepEqual(result.instance.config.serverToken, { kind: 'env', name: 'FRM_SERVER_TOKEN' });
  assert.deepEqual(result.instance.config.localToken, { kind: 'env', name: 'FRM_LOCAL_TOKEN' });
  assert.equal(result.instance.localToken, 'synthetic-local-token');
});

test('listen overrides suppress invalid values without losing other lookups', { timeout: 3000 }, async context => {
  const saved = '{"remote":"http://127.0.0.1:43209","port":0}\n';
  const state = await fixture(context, { FrM_Remote: 'http://127.0.0.1:43210',
    FrM_Host: 'invalid/host', FrM_Port: 'invalid-port' }, saved);
  const result = await state.resolve({ host: '127.0.0.1', port: 0 });
  assert.equal(result.error, undefined);
  assert.equal(result.instance.config.remote, 'http://127.0.0.1:43210');
  assert.equal(result.instance.config.host, '127.0.0.1');
  assert.equal(result.instance.config.port, 0);
});

test('uppercase environment settings and explicit listen overrides still work', { timeout: 3000 }, async context => {
  const state = await fixture(context, { FRM_REMOTE: 'http://127.0.0.1:43210',
    FRM_HOST: 'invalid/host', FRM_PORT: 'invalid-port', FRM_LOCAL_TOKEN: 'synthetic-local-token' });
  const result = await state.resolve({ host: '127.0.0.1', port: 0 });
  assert.equal(result.error, undefined);
  assert.equal(result.instance.config.remote, 'http://127.0.0.1:43210');
  assert.equal(result.instance.config.host, '127.0.0.1');
  assert.equal(result.instance.config.port, 0);
  assert.equal(result.instance.localToken, 'synthetic-local-token');
});
