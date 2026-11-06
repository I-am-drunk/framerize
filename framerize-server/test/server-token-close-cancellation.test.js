import assert from 'node:assert/strict';
import test from 'node:test';
import * as files from 'node:fs/promises';
import { getEventListeners } from 'node:events';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, version as esbuildVersion } from 'esbuild';

const hookKey = 'framerize.test.serverTokenClose';
const root = fileURLToPath(new URL('../../', import.meta.url));
const digest = value => createHash('sha256').update(value).digest('hex');
const compiled = await build({ absWorkingDir: root,
  entryPoints: ['framerize-server/src/config.ts'], bundle: true,
  platform: 'node', format: 'esm', write: false, metafile: true,
  plugins: [{ name: 'owned-token-file', setup(builder) {
    builder.onResolve({ filter: /^node:fs\/promises$/ }, args =>
      args.namespace === 'file' ? { path: 'owned-files', namespace: 'fixture' } : undefined);
    builder.onLoad({ filter: /.*/, namespace: 'fixture' }, () => ({
      contents: `export const open = (...args) => globalThis[Symbol.for('${hookKey}')](...args);`,
      loader: 'js',
    }));
  } }],
});
globalThis[Symbol.for(hookKey)] = files.open;
const bundle = compiled.outputFiles[0].contents;
const { loadConfig, readServerToken } = await import(
  `data:text/javascript;base64,${Buffer.from(bundle).toString('base64')}`);
const sources = {};
for (const path of Object.keys(compiled.metafile.inputs).filter(path => path.endsWith('.ts'))) {
  sources[path] = digest(await files.readFile(join(root, path)));
}
console.log(JSON.stringify({ evidence: 'source token loader with delayed private file results',
  runtime: process.version, esbuildVersion, sources, bundle: digest(bundle),
  test: digest(await files.readFile(new URL(import.meta.url))) }));

async function bounded(promise, label) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolveUnused, reject) => {
      timer = setTimeout(() => reject(new Error(label)), 1500);
    })]);
  } finally { clearTimeout(timer); }
}

const token = 'synthetic-token-file-close';

async function fixture(context, stage) {
  const home = await files.mkdtemp(join(tmpdir(), 'frm-token-close-'));
  const path = join(home, 'token');
  const entered = Promise.withResolvers();
  const release = Promise.withResolvers();
  const closed = Promise.withResolvers();
  let handle, closing, closes = 0, cleaned = false;
  const cleanup = async () => {
    if (cleaned) return;
    release.resolve();
    if (handle && handle.fd !== -1) await handle.close();
    globalThis[Symbol.for(hookKey)] = files.open;
    await files.rm(home, { recursive: true, force: true });
    await assert.rejects(files.access(home), { code: 'ENOENT' });
    cleaned = true;
  };
  context.after(cleanup);
  await files.writeFile(path, `${token}\n`, { mode: 0o600 });
  globalThis[Symbol.for(hookKey)] = async (...args) => {
    assert.equal(args[0], path, 'Only the synthetic token file may be opened');
    handle = await files.open(...args);
    const read = async (method, parameters) => {
      const result = await handle[method](...parameters);
      if (stage === method) { entered.resolve(); await release.promise; }
      return result;
    };
    return {
      stat: (...parameters) => read('stat', parameters),
      read: (...parameters) => read('read', parameters),
      close() {
        closes++;
        closing = handle.close().then(() => { closed.resolve(); });
        return closing;
      },
    };
  };
  return { entered, release, closed, cleanup,
    config: loadConfig({ serverTokenFile: path }, {}),
    get closing() { return closing; },
    get count() { return closes; },
    get descriptorClosed() { return handle?.fd === -1; },
    unchanged: async () => (await files.readFile(path, 'utf8')) === `${token}\n`,
  };
}

for (const stage of ['stat', 'read']) {
  test(`token cancellation closes the file before delayed ${stage} settles`, { timeout: 5000 }, async context => {
    const state = await fixture(context, stage);
    const controller = new AbortController();
    const reason = new Error('Synthetic token stop');
    const pending = readServerToken(state.config, controller.signal)
      .then(value => ({ value }), error => ({ error }));
    await bounded(state.entered.promise, 'Token file operation was not reached');
    controller.abort(reason);
    const outcome = await bounded(pending, 'Token loading did not cancel');
    if (state.closing) await bounded(state.closing, 'Token file close did not finish');
    const beforeRelease = { closes: state.count, descriptorClosed: state.descriptorClosed };
    state.release.resolve();
    await bounded(state.closed.promise, 'Released token work did not close the file');
    const unchanged = await state.unchanged();
    const closes = state.count;
    const listeners = getEventListeners(controller.signal, 'abort').length;
    await state.cleanup();
    context.diagnostic(JSON.stringify({ stage, beforeRelease, closes, listeners,
      unchanged, privateStateRemoved: true }));
    assert.equal(outcome.error, reason);
    assert.equal(outcome.value, undefined);
    assert.equal(unchanged, true);
    assert.equal(closes, 1);
    assert.equal(listeners, 0);
    assert.equal(beforeRelease.descriptorClosed, true, 'Do not retain an acquired file after cancellation');
  });
}

for (const withSignal of [false, true]) {
  test(`ordinary token read closes its file ${withSignal ? 'with' : 'without'} a signal`, async context => {
    const state = await fixture(context, 'none');
    const controller = new AbortController();
    const value = await bounded(readServerToken(state.config, withSignal ? controller.signal : undefined),
      'Ordinary token read did not settle');
    const unchanged = await state.unchanged();
    const closed = state.descriptorClosed;
    const closes = state.count;
    const listeners = getEventListeners(controller.signal, 'abort').length;
    await state.cleanup();
    context.diagnostic(JSON.stringify({ stage: 'control', withSignal, closed, closes,
      listeners, unchanged, privateStateRemoved: true }));
    assert.equal(value, token);
    assert.equal(unchanged, true);
    assert.equal(closed, true);
    assert.equal(closes, 1);
    assert.equal(listeners, 0);
  });
}
