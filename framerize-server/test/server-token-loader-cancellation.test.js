import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/config.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
stop();
const bytes = compiled.outputFiles[0].contents;
const bundleHash = createHash('sha256').update(bytes).digest('hex');
const { loadConfig, readServerToken } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
async function bounded(pending, milliseconds = 1500) {
  let timer;
  try { return await Promise.race([pending, new Promise((resolveUnused, reject) => {
    timer = setTimeout(() => reject(new Error('Owned fixture wait expired')), milliseconds);
  })]); } finally { clearTimeout(timer); }
}

for (const phase of ['open', 'stat', 'read', 'close', 'open-reject', 'read-reject']) {
  test(`token cancellation settles before delayed ${phase} and cleans late work`, { timeout: 5000 }, async context => {
    const root = await fs.mkdtemp(join(tmpdir(), 'token-loader-cancel-'));
    const path = join(root, 'private-token');
    const controller = new AbortController();
    const reason = new Error('Synthetic token cancellation');
    const originalOpen = fs.open;
    let release, entered, finished, handle, pending, mock;
    let closes = 0;
    const held = new Promise(resolve => { release = resolve; });
    const started = new Promise(resolve => { entered = resolve; });
    const cleaned = new Promise(resolve => { finished = resolve; });
    context.after(async () => {
      controller.abort(reason);
      release();
      try { if (pending) await bounded(pending); }
      finally {
        if (handle && handle.fd !== -1) await handle.close();
        mock?.mock.restore();
        syncBuiltinESMExports();
        await fs.rm(root, { recursive: true, force: true });
        await assert.rejects(fs.stat(root), { code: 'ENOENT' });
      }
    });
    const token = 'synthetic-loader-token\n';
    await fs.writeFile(path, token, { mode: 0o600 });
    const hold = async () => { entered(); await held; };
    mock = context.mock.method(fs, 'open', async (requested, ...args) => {
      if (requested !== path) return originalOpen(requested, ...args);
      if (phase.startsWith('open')) await hold();
      if (phase === 'open-reject') { finished(); throw new Error('Synthetic late open failure'); }
      handle = await originalOpen(requested, ...args);
      const originalClose = handle.close.bind(handle);
      context.mock.method(handle, 'close', async () => {
        if (phase === 'close') await hold();
        await originalClose();
        closes++;
        finished();
      });
      for (const method of ['stat', 'read']) {
        const original = handle[method].bind(handle);
        context.mock.method(handle, method, async (...parameters) => {
          if (phase.startsWith(method)) await hold();
          if (phase === `${method}-reject`) throw new Error(`Synthetic late ${method} failure`);
          return original(...parameters);
        });
      }
      return handle;
    });
    syncBuiltinESMExports();
    pending = readServerToken(loadConfig({ serverTokenFile: path }, {}), controller.signal)
      .then(value => ({ value }), error => ({ error }));
    await bounded(started);
    controller.abort(reason);
    let early;
    try { early = await bounded(pending, 400); }
    finally { release(); }
    await bounded(cleaned);
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(early.error, reason);
    assert.equal(early.value, undefined);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    assert.equal(closes, phase === 'open-reject' ? 0 : 1);
    if (handle) assert.equal(handle.fd, -1);
    assert.equal(await fs.readFile(path, 'utf8'), token);
    context.diagnostic(JSON.stringify({ phase, bundleHash, cancelledBeforeRelease: true,
      closes, descriptor: handle?.fd ?? null, abortListeners: 0 }));
  });
}

test('already aborted token loading never opens a file or returns an inline token', async context => {
  const controller = new AbortController();
  const reason = new Error('Synthetic early cancellation');
  controller.abort(reason);
  let opens = 0;
  const mock = context.mock.method(fs, 'open', async () => {
    opens++;
    throw new Error('No file may open after cancellation');
  });
  syncBuiltinESMExports();
  try {
    for (const config of [loadConfig({ serverTokenFile: '/synthetic/unused' }, {}),
      loadConfig({ serverToken: 'synthetic-inline-token' }, {})]) {
      await assert.rejects(readServerToken(config, controller.signal), error => error === reason);
    }
    assert.equal(opens, 0);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  } finally { mock.mock.restore(); syncBuiltinESMExports(); }
});

test('uncancelled token loading retains file validation and inline behavior', async context => {
  const root = await fs.mkdtemp(join(tmpdir(), 'token-loader-control-'));
  context.after(async () => {
    await fs.rm(root, { recursive: true, force: true });
    await assert.rejects(fs.stat(root), { code: 'ENOENT' });
  });
  const path = join(root, 'private-token');
  const config = loadConfig({ serverTokenFile: path }, {});
  const controller = new AbortController();
  await fs.writeFile(path, 'synthetic-normal-token\n', { mode: 0o600 });
  assert.equal(await readServerToken(config), 'synthetic-normal-token');
  assert.equal(await readServerToken(config, controller.signal), 'synthetic-normal-token');
  assert.equal(await readServerToken(loadConfig({ serverToken: 'synthetic-inline' }, {})), 'synthetic-inline');
  assert.equal(await readServerToken(loadConfig({}, {}), controller.signal), '');
  await fs.writeFile(path, 'invalid token content');
  await assert.rejects(readServerToken(config, controller.signal), {
    message: 'Server token file unavailable',
  });
  await fs.writeFile(path, 'a'.repeat(16_385));
  await assert.rejects(readServerToken(config), { message: 'Server token file unavailable' });
  await fs.rm(path);
  await assert.rejects(readServerToken(config, controller.signal), { message: 'Server token file unavailable' });
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});
