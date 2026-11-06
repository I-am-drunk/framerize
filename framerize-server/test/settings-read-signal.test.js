import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build, stop } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/settings.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
stop();
const bytes = compiled.outputFiles[0].contents;
const bundleHash = createHash('sha256').update(bytes).digest('hex');
const { readSettings, configuredSettings } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

async function fixture(context) {
  const root = await fs.mkdtemp(join(tmpdir(), 'settings-read-signal-'));
  context.after(async () => {
    context.mock.restoreAll();
    syncBuiltinESMExports();
    await fs.rm(root, { recursive: true, force: true });
    await assert.rejects(fs.stat(root), { code: 'ENOENT' });
  });
  const path = join(root, 'server.json');
  const saved = { version: 1, config: { port: 8888 }, environment: { SYNTHETIC: 'saved' } };
  await fs.writeFile(path, JSON.stringify(saved), { mode: 0o600 });
  return { path, saved };
}

test('pre-aborted settings operations do not read a file', async context => {
  const { path } = await fixture(context);
  const controller = new AbortController();
  const reason = new Error('cancel before settings read');
  controller.abort(reason);
  let reads = 0;
  context.mock.method(fs, 'open', async () => { reads++; throw new Error('Unexpected open'); });
  syncBuiltinESMExports();
  await assert.rejects(readSettings(path, controller.signal), error => error === reason);
  await assert.rejects(configuredSettings(undefined, {}, {}, controller.signal), error => error === reason);
  assert.equal(reads, 0);
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});

for (const useSignal of [false, true]) {
  test(`settings success preserves contents and overrides, signal ${useSignal}`, async context => {
    const { path, saved } = await fixture(context);
    const controller = new AbortController();
    const signal = useSignal ? controller.signal : undefined;
    assert.deepEqual(await readSettings(path, signal), saved);
    const result = await configuredSettings(path, { port: 9001 }, { SYNTHETIC: 'run' }, signal);
    assert.equal(result.config.port, 9001);
    assert.equal(result.env.SYNTHETIC, 'run');
    assert.equal(await fs.readFile(path, 'utf8'), JSON.stringify(saved));
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    context.diagnostic(`source bundle sha256 ${bundleHash}`);
  });
}

test('non-cancelled read and JSON failures retain the configuration diagnostic', async context => {
  const { path } = await fixture(context);
  const controller = new AbortController();
  const expected = { name: 'ConfigurationError', message: 'Server settings are unavailable or invalid' };
  await fs.writeFile(path, '{invalid');
  await assert.rejects(readSettings(path, controller.signal), expected);
  await assert.rejects(readSettings(`${path}.missing`, controller.signal), expected);
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});

async function within(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolveUnused, reject) => {
      timer = setTimeout(() => reject(new Error('Settings operation did not settle')), 1000);
    })]);
  } finally { clearTimeout(timer); }
}

for (const late of ['success', 'failure']) {
  test(`cancellation wins over late read ${late} and removes its listener`, { timeout: 5000 }, async context => {
    const { path } = await fixture(context);
    const originalRead = fs.readFile;
    const originalOpen = fs.open;
    const controller = new AbortController();
    const entered = Promise.withResolvers();
    const release = Promise.withResolvers();
    const finished = Promise.withResolvers();
    const reason = new Error('cancel pending settings read');
    context.mock.method(fs, 'open', async function (file, flags) {
      const handle = await originalOpen.call(this, file, flags);
      const read = handle.readFile.bind(handle);
      context.mock.method(handle, 'readFile', async options => {
        try {
          assert.equal(options.signal, controller.signal);
          const text = await read(options);
          entered.resolve();
          await release.promise;
          if (late === 'failure') throw new Error('late synthetic file failure');
          return text;
        } finally { finished.resolve(); }
      });
      return handle;
    });
    syncBuiltinESMExports();
    const pending = readSettings(path, controller.signal).then(value => ({ value }), error => ({ error }));
    try {
      await within(entered.promise);
      controller.abort(reason);
      assert.equal((await within(pending)).error, reason);
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    } finally { release.resolve(); await within(finished.promise); }
    assert.equal(await originalRead(path, 'utf8'), JSON.stringify({ version: 1,
      config: { port: 8888 }, environment: { SYNTHETIC: 'saved' } }));
  });
}
