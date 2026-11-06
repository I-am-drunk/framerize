import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/settings.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
stop();
const { defaultSettingsPath } = await import(`data:text/javascript;base64,${
  Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function privateHome(context) {
  const root = await fs.mkdtemp(join(tmpdir(), 'default-settings-controls-'));
  context.after(() => fs.rm(root, { recursive: true, force: true }));
  return { root, env: { FRAMERIZE_SERVER_HOME: root } };
}

function replaceAccess(context, callback) {
  const mock = context.mock.method(fs, 'access', callback);
  syncBuiltinESMExports();
  context.after(() => { context.mock.restoreAll(); syncBuiltinESMExports(); });
  return mock;
}

test('default lookup preserves present, missing and denied results', async context => {
  const { root, env } = await privateHome(context);
  const path = join(root, 'server.json');
  const controller = new AbortController();
  await fs.writeFile(path, '{}', { mode: 0o600 });
  for (const signal of [undefined, controller.signal]) {
    assert.equal(await defaultSettingsPath(env, signal), path);
    assert.equal(await defaultSettingsPath({}, signal), undefined);
  }
  await fs.unlink(path);
  for (const signal of [undefined, controller.signal]) {
    assert.equal(await defaultSettingsPath(env, signal), undefined);
  }
  replaceAccess(context, async () => { throw Object.assign(new Error('synthetic denial'), { code: 'EACCES' }); });
  for (const signal of [undefined, controller.signal]) {
    await assert.rejects(defaultSettingsPath(env, signal),
      { message: 'Server settings are unavailable or invalid' });
  }
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});

test('pre-aborted default lookup starts no filesystem request', async context => {
  const { env } = await privateHome(context);
  const controller = new AbortController();
  const reason = new Error('stop before lookup');
  controller.abort(reason);
  const access = replaceAccess(context, async () => assert.fail('No access may start after abort'));
  await assert.rejects(defaultSettingsPath(env, controller.signal), error => error === reason);
  assert.equal(access.mock.calls.length, 0);
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});

test('cancelled lookup preserves its reason and handles a late access failure', async context => {
  const { env } = await privateHome(context);
  const controller = new AbortController();
  const reason = new Error('stop pending lookup');
  const held = Promise.withResolvers();
  context.after(() => held.resolve());
  const access = replaceAccess(context, () => held.promise);
  const pending = defaultSettingsPath(env, controller.signal);
  const rejected = assert.rejects(pending, error => error === reason);
  controller.abort(reason);
  await rejected;
  assert.equal(access.mock.calls.length, 1);
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  held.reject(Object.assign(new Error('late failure'), { code: 'EACCES' }));
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});
