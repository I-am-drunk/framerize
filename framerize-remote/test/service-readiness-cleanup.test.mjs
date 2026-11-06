import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { connect } from 'node:net';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/serviceReadiness.ts', import.meta.url))],
  alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) },
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { waitForServiceListener } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function bounded(promise, milliseconds, message) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(message)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function refused(port) {
  const socket = connect({ host: '127.0.0.1', port });
  try {
    const result = await bounded(new Promise(resolve => {
      socket.once('error', error => resolve(error.code));
      socket.once('connect', () => resolve('connected'));
    }), 1000, 'port-refusal check timed out');
    assert.equal(result, 'ECONNREFUSED');
  } finally { socket.destroy(); }
}

async function fixture(context, handler) {
  const received = Promise.withResolvers();
  const closed = Promise.withResolvers();
  const sockets = new Set();
  const paths = [];
  const server = createServer((request, response) => {
    paths.push(request.url);
    request.socket.once('close', closed.resolve);
    received.resolve();
    handler(response, paths.length);
  });
  server.on('connection', socket => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
  });
  let port;
  context.after(async () => {
    for (const socket of sockets) socket.destroy();
    await bounded(new Promise(resolve => server.close(resolve)), 1000, 'fixture close timed out');
    if (port !== undefined) await refused(port);
  });
  await bounded(new Promise(resolve => server.listen(0, '127.0.0.1', resolve)), 1000, 'fixture listen timed out');
  port = server.address().port;
  return { port, received: received.promise, closed: closed.promise, paths, sockets };
}

for (const partialBody of [false, true]) {
  test(`service-query rejection closes pending health ${partialBody ? 'body' : 'headers'}`, { timeout: 5000 }, async context => {
    const peer = await fixture(context, response => {
      if (partialBody) {
        response.writeHead(200, { 'x-framerize-remote-pid': '4242', 'content-length': '100' });
        response.write('{"status":');
      }
    });
    const failure = new Error('synthetic service query failed');
    let queries = 0;
    const waiting = waitForServiceListener({ host: '127.0.0.1', port: peer.port, timeoutMs: 4000,
      service: async () => {
        queries += 1;
        await peer.received;
        throw failure;
      },
    });
    await assert.rejects(bounded(waiting, 1000, 'service rejection did not propagate'), error => error === failure);
    await bounded(peer.closed, 1000, 'health socket survived the service-query rejection');
    assert.equal(peer.sockets.size, 0);
    assert.deepEqual(peer.paths, ['/health']);
    assert.equal(queries, 1);
  });
}

test('a mismatched PID still retries and returns the matching service PID', { timeout: 5000 }, async context => {
  const peer = await fixture(context, (response, attempt) => {
    response.writeHead(200, { 'x-framerize-remote-pid': String(attempt === 1 ? 4241 : 4242) });
    response.end('{"status":"ok"}');
  });
  let queries = 0;
  const pid = await waitForServiceListener({ host: '127.0.0.1', port: peer.port, timeoutMs: 3000,
    service: async () => { queries += 1; return { running: true, pid: 4242 }; },
  });
  assert.equal(pid, 4242);
  assert.equal(queries, 2);
  assert.deepEqual(peer.paths, ['/health', '/health']);
});

test('caller cancellation retains its diagnostic and closes the health socket', { timeout: 5000 }, async context => {
  const peer = await fixture(context, () => {});
  const controller = new AbortController();
  const waiting = waitForServiceListener({ host: '127.0.0.1', port: peer.port, timeoutMs: 4000,
    signal: controller.signal, service: async () => ({ running: true, pid: 4242 }),
  });
  const rejected = assert.rejects(waiting, { code: 'aborted' });
  await bounded(peer.received, 1000, 'health request not received');
  controller.abort();
  await bounded(rejected, 1000, 'caller cancellation did not settle');
  await bounded(peer.closed, 1000, 'health socket survived caller cancellation');
  assert.equal(peer.sockets.size, 0);
  assert.deepEqual(peer.paths, ['/health']);
});
