import assert from 'node:assert/strict';
import test from 'node:test';
import * as files from 'node:fs/promises';
import http from 'node:http';
import { connect } from 'node:net';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, version as esbuildVersion } from 'esbuild';

const hookKey = 'framerize.test.providerCredentialOpen';
const root = fileURLToPath(new URL('../../', import.meta.url));
const digest = value => createHash('sha256').update(value).digest('hex');
const compiled = await build({ absWorkingDir: root,
  entryPoints: ['framerize-server/src/provider.ts'], bundle: true,
  platform: 'node', format: 'esm', write: false, metafile: true,
  plugins: [{ name: 'owned-credential-file', setup(builder) {
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
const { createProvider } = await import(`data:text/javascript;base64,${Buffer.from(bundle).toString('base64')}`);
const sources = {};
for (const path of Object.keys(compiled.metafile.inputs).filter(path => path.endsWith('.ts'))) {
  sources[path] = digest(await files.readFile(join(root, path)));
}
console.log(JSON.stringify({ evidence: 'source API with delayed private credential operations',
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

const source = async function* () { yield Buffer.from('{}'); };
const nextTurn = () => new Promise(resolve => setImmediate(resolve));

async function fixture(context, stage) {
  const home = await files.mkdtemp(join(tmpdir(), 'frm-provider-credential-'));
  const path = join(home, 'authorization');
  const entered = Promise.withResolvers();
  const release = Promise.withResolvers();
  const closed = Promise.withResolvers();
  const sockets = new Set();
  const requests = [];
  const metrics = { stat: 0, read: 0, close: 0 };
  let handle, provider, closeWork, cleaned = false;
  const server = http.createServer((request, response) => {
    requests.push({ authorized: request.headers.authorization === 'Bearer SYNTHETIC_FILE_TOKEN' });
    request.resume(); response.end('ok');
  });
  server.on('connection', socket => {
    sockets.add(socket); socket.once('close', () => sockets.delete(socket));
  });
  const cleanup = async () => {
    if (cleaned) return;
    release.resolve();
    await provider?.close();
    await handle?.close();
    for (const socket of sockets) socket.destroy();
    await new Promise(resolve => server.close(resolve));
    globalThis[Symbol.for(hookKey)] = files.open;
    await files.rm(home, { recursive: true, force: true });
    await assert.rejects(files.access(home), { code: 'ENOENT' });
    cleaned = true;
  };
  context.after(cleanup);
  await files.writeFile(path, 'SYNTHETIC_FILE_TOKEN\n', { mode: 0o600 });
  const pause = async target => {
    if (stage === target) { entered.resolve(); await release.promise; }
  };
  globalThis[Symbol.for(hookKey)] = async (...args) => {
    assert.equal(args[0], path, 'Only the private credential file may be opened');
    handle = await files.open(...args);
    await pause('open');
    return {
      async stat() { metrics.stat++; const result = await handle.stat(); await pause('stat'); return result; },
      async read(...readArgs) {
        metrics.read++; const result = await handle.read(...readArgs); await pause('read'); return result;
      },
      close() {
        metrics.close++;
        closeWork = handle.close().then(() => { closed.resolve(); });
        return closeWork;
      },
    };
  };
  await bounded(new Promise((resolve, reject) => {
    server.once('error', reject); server.listen(0, '127.0.0.1', resolve);
  }), 'Credential fixture did not listen');
  const port = server.address().port;
  provider = createProvider({ upstream: `http://127.0.0.1:${port}`, authorizationFile: path,
    headersTimeoutMs: 5000, idleTimeoutMs: 1000 });
  return { provider, entered, release, closed, requests, metrics, cleanup,
    get descriptorClosed() { return handle?.fd === -1; },
    get closeWork() { return closeWork; },
    async verifyCleanup() {
      await cleanup();
      await assert.rejects(new Promise((resolve, reject) => {
        const socket = connect({ host: '127.0.0.1', port });
        socket.once('connect', () => { socket.destroy(); resolve(); });
        socket.once('error', reject);
        socket.setTimeout(1000, () => socket.destroy(new Error('Fixture port probe timed out')));
      }), { code: 'ECONNREFUSED' });
    } };
}

for (const stage of ['open', 'stat', 'read']) for (const action of ['abort', 'close']) {
  test(`${action} releases a credential file during ${stage}`, { timeout: 6000 }, async context => {
    const state = await fixture(context, stage);
    const controller = new AbortController();
    const operation = state.provider.forward({ path: '/v1/responses', body: source(),
      signal: controller.signal }).then(result => ({ result }), error => ({ error }));
    await bounded(state.entered.promise, 'The credential stage was not reached');
    if (action === 'abort') controller.abort();
    else await bounded(state.provider.close(), 'Provider shutdown did not settle');
    const outcome = await bounded(operation, 'Cancelled credential request did not settle');
    await nextTurn();
    if (state.closeWork) await bounded(state.closeWork, 'Acquired credential file did not close');
    const beforeRelease = { ...state.metrics, descriptorClosed: state.descriptorClosed };
    state.release.resolve();
    await bounded(state.closed.promise, 'The released credential file did not close');
    const afterRelease = { ...state.metrics, descriptorClosed: state.descriptorClosed };
    await state.verifyCleanup();
    context.diagnostic(JSON.stringify({ stage, action, code: outcome.error?.code,
      beforeRelease, afterRelease, requests: state.requests.length, privateStateRemoved: true }));
    assert.equal(outcome.error?.code, action === 'abort' ? 'request_cancelled' : 'provider_closed');
    assert.equal(state.requests.length, 0);
    assert.equal(afterRelease.close, 1, 'The owned credential file closes exactly once');
    if (stage === 'open') assert.equal(afterRelease.stat, 0, 'Late opens must not start metadata reads');
    else assert.equal(beforeRelease.descriptorClosed, true, 'Cancellation closes an acquired credential file');
  });
}

test('ordinary credential files still authorize and close', { timeout: 6000 }, async context => {
  const state = await fixture(context, 'none');
  const result = await bounded(state.provider.forward({ path: '/v1/responses', body: source() }),
    'The control request did not settle');
  const chunks = [];
  for await (const chunk of result.body) chunks.push(chunk);
  const metrics = { ...state.metrics, descriptorClosed: state.descriptorClosed };
  await state.verifyCleanup();
  context.diagnostic(JSON.stringify({ stage: 'control', metrics,
    requests: state.requests.length, privateStateRemoved: true }));
  assert.equal(result.status, 200);
  assert.equal(Buffer.concat(chunks).toString(), 'ok');
  assert.deepEqual(state.requests, [{ authorized: true }]);
  assert.equal(metrics.close, 1);
  assert.equal(metrics.descriptorClosed, true);
});
