import test from 'node:test';
import assert from 'node:assert/strict';
import net from 'node:net';
import { once } from 'node:events';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/remoteClient.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { sendRequest } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function bounded(pending, milliseconds = 1500) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error('Fixture deadline exceeded')), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function refused(port) {
  const socket = net.connect({ host: '127.0.0.1', port });
  try {
    const error = await bounded(new Promise(resolve => {
      socket.once('error', resolve);
      socket.once('connect', () => resolve(new Error('Listener remains open')));
    }));
    assert.equal(error.code, 'ECONNREFUSED');
  } finally { socket.destroy(); }
}

async function fixture(context, upgrade) {
  const controller = new AbortController();
  const sockets = new Set();
  let port;
  let closed;
  const peerClosed = new Promise(resolve => { closed = resolve; });
  const server = net.createServer(socket => {
    sockets.add(socket);
    socket.on('error', () => {});
    socket.once('close', () => { sockets.delete(socket); closed(); });
    socket.once('data', () => {
      if (upgrade) socket.write('HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: synthetic\r\n\r\n');
      else socket.end('HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}');
    });
  });
  context.after(async () => {
    controller.abort();
    for (const socket of sockets) socket.destroy();
    await bounded(new Promise(resolve => server.close(resolve)));
    if (port !== undefined) await refused(port);
    context.diagnostic('Owned sockets closed and fixture port refused.');
  });
  server.listen(0, '127.0.0.1');
  await bounded(once(server, 'listening'));
  port = server.address().port;
  return { controller, peerClosed, origin: `http://127.0.0.1:${port}` };
}

function requestOptions(state, method) {
  return { origin: state.origin, path: '/synthetic', method, token: null, headers: {},
    signal: state.controller.signal, headersTimeoutMs: 2000, idleTimeoutMs: 2000,
    ...(method === 'POST' ? { body: (async function* () { yield Buffer.from('{}'); })() } : {}) };
}

for (const method of ['GET', 'POST']) {
  test(`${method} rejects an unsupported upgrade when its socket closes`, { timeout: 5000 }, async context => {
    const state = await fixture(context, true);
    const pending = sendRequest(requestOptions(state, method)).then(
      result => { result.dispose(); return { response: true }; }, error => ({ error }));
    await bounded(state.peerClosed);
    const settledBeforeDeadline = await bounded(pending.then(() => true), 500).catch(() => false);
    if (!settledBeforeDeadline) state.controller.abort();
    const result = await bounded(pending);
    context.diagnostic(JSON.stringify({ method, peerClosed: true, settledBeforeDeadline,
      code: result.error?.code, response: result.response === true }));
    assert.equal(settledBeforeDeadline, true, 'A closed request must not wait for its headers deadline.');
    assert.equal(result.error?.code, 'connection_failed');
  });
}

test('ordinary response remains readable after request close', { timeout: 5000 }, async context => {
  const state = await fixture(context, false);
  const result = await bounded(sendRequest(requestOptions(state, 'GET')));
  context.after(result.dispose);
  const chunks = [];
  for await (const chunk of result.message) chunks.push(chunk);
  assert.equal(result.message.statusCode, 200);
  assert.equal(Buffer.concat(chunks).toString(), '{}');
  result.dispose();
  await bounded(state.peerClosed);
});
