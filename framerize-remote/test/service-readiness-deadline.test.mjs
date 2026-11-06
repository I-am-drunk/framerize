import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { connect } from 'node:net';
import { once } from 'node:events';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/serviceReadiness.ts', import.meta.url))],
  alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) },
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
});
const bytes = compiled.outputFiles[0].contents;
const bundleSha256 = createHash('sha256').update(bytes).digest('hex');
const { waitForServiceListener } = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

async function bounded(promise, milliseconds = 1000) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error('fixture exceeded its bound')), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function portState(port) {
  const socket = connect({ host: '127.0.0.1', port });
  try {
    return await bounded(new Promise(resolve => {
      socket.once('connect', () => resolve('connected'));
      socket.once('error', error => resolve(error.code));
    }));
  } finally { socket.destroy(); }
}

async function fixture(context) {
  const sockets = new Set();
  const requested = Promise.withResolvers();
  const server = createServer((request, response) => {
    response.writeHead(200, { 'x-framerize-remote-pid': '4242', connection: 'close' });
    response.end('{"status":"ok"}');
    requested.resolve(request.url);
  });
  server.on('connection', socket => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
  });
  let port, closed = false;
  const cleanup = async () => {
    if (closed) return;
    closed = true;
    for (const socket of sockets) socket.destroy();
    await bounded(new Promise(resolve => server.close(resolve)));
    if (port !== undefined) assert.equal(await portState(port), 'ECONNREFUSED');
  };
  context.after(cleanup);
  server.listen(0, '127.0.0.1');
  await bounded(once(server, 'listening'));
  port = server.address().port;
  return { port, requested: requested.promise, cleanup };
}

function pendingQuery() {
  const reached = Promise.withResolvers(), completion = Promise.withResolvers();
  let receivedSignal, ended = false, wasAborted = false;
  const finish = () => {
    if (ended) return;
    ended = true;
    receivedSignal?.removeEventListener('abort', abort);
    completion.resolve({ running: true, pid: 4242 });
  };
  const abort = () => { wasAborted = true; finish(); };
  return { reached: reached.promise, finish,
    get ended() { return ended; }, get wasAborted() { return wasAborted; },
    service(signal) {
      receivedSignal = signal;
      if (signal?.aborted) abort();
      else signal?.addEventListener('abort', abort, { once: true });
      reached.resolve();
      return completion.promise;
    },
  };
}

for (const mode of ['deadline', 'caller abort']) {
  test(`pending service query observes ${mode} before readiness settles`, { timeout: 5000 }, async context => {
    const query = pendingQuery(), controller = new AbortController();
    let pending;
    context.after(async () => { query.finish(); controller.abort(); if (pending) await bounded(pending); });
    const server = await fixture(context);
    const observed = { settled: false };
    pending = waitForServiceListener({ host: '127.0.0.1', port: server.port,
      timeoutMs: mode === 'deadline' ? 100 : 3000, signal: controller.signal,
      service: query.service }).then(value => {
        observed.settled = true; observed.value = value;
      }, error => { observed.settled = true; observed.code = error.code; });
    await bounded(query.reached);
    if (mode === 'caller abort') controller.abort();
    let timer;
    try {
      await Promise.race([pending, new Promise(resolve => { timer = setTimeout(resolve, 700); })]);
    } finally { clearTimeout(timer); }
    const beforeRelease = { ...observed, queryEnded: query.ended, queryAborted: query.wasAborted };
    query.finish();
    await bounded(pending);
    await server.cleanup();
    context.diagnostic(JSON.stringify({ mode, beforeRelease, afterRelease: observed,
      node: process.version, bundleSha256, fixturePort: 'ECONNREFUSED', syntheticService: true }));
    assert.equal(beforeRelease.settled, true, 'readiness must settle without manually releasing the service query');
    assert.equal(beforeRelease.queryEnded, true, 'the service query must finish before readiness settles');
    assert.equal(beforeRelease.queryAborted, true, 'the service query must receive cancellation');
    assert.equal(beforeRelease.code, mode === 'deadline' ? 'service_not_ready' : 'aborted');
  });
}

test('a responsive service query still verifies the listener PID', { timeout: 3000 }, async context => {
  const server = await fixture(context);
  const result = await bounded(waitForServiceListener({ host: '127.0.0.1', port: server.port,
    timeoutMs: 1000, service: async () => ({ running: true, pid: 4242 }) }));
  assert.equal(result, 4242);
  assert.equal(await bounded(server.requested), '/health');
  await server.cleanup();
  context.diagnostic(JSON.stringify({ node: process.version, bundleSha256, fixturePort: 'ECONNREFUSED', syntheticService: true }));
});
