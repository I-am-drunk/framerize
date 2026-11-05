import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import net from 'node:net';
import { once } from 'node:events';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { join } from 'node:path';
import { build } from 'esbuild';

async function loadBackend() {
  const installed = process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT;
  if (installed) {
    const core = await import('../framerize-core/dist/index.js');
    const remote = await import(pathToFileURL(join(installed, 'dist/index.js')).href);
    return { createInstance: core.createInstance, createRemoteBackend: remote.createRemoteBackend };
  }
  const root = fileURLToPath(new URL('../', import.meta.url));
  const compiled = await build({ stdin: { resolveDir: root, contents:
    "export { createInstance } from './framerize-core/src/index.ts'; export { createRemoteBackend } from './framerize-remote/src/remoteBackend.ts';" },
    alias: { '@framerize/core': join(root, 'framerize-core/src/index.ts') },
    bundle: true, platform: 'node', format: 'esm', write: false });
  return import(
    `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
}
const { createInstance, createRemoteBackend } = await loadBackend();

async function bounded(pending, milliseconds = 4000) {
  let timer;
  try { return await Promise.race([pending, new Promise((resolve, reject) => {
    timer = setTimeout(() => reject(new Error('Fixture deadline exceeded')), milliseconds);
  })]); } finally { clearTimeout(timer); }
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

async function fixture(context) {
  const sockets = new Set();
  const requests = [];
  let instance, backend, remotePort, upstreamPort, client;
  const upstream = net.createServer(socket => {
    sockets.add(socket);
    socket.on('error', () => {});
    socket.once('close', () => sockets.delete(socket));
    let received = '';
    const headers = chunk => {
      received += chunk.toString();
      if (!received.includes('\r\n\r\n')) return;
      socket.off('data', headers);
      requests.push(received.split('\r\n', 1)[0]);
      socket.write('HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: synthetic\r\n\r\n');
    };
    socket.on('data', headers);
  });
  context.after(async () => {
    client?.destroy();
    try { if (instance) await bounded(instance.close()); else backend?.close(); }
    finally {
      for (const socket of sockets) socket.destroy();
      await bounded(new Promise(resolve => upstream.close(resolve)));
    }
    if (remotePort !== undefined) await refused(remotePort);
    if (upstreamPort !== undefined) await refused(upstreamPort);
    context.diagnostic('Owned Remote and upstream ports both refuse connections.');
  });
  upstream.listen(0, '127.0.0.1');
  await bounded(once(upstream, 'listening'));
  upstreamPort = upstream.address().port;
  backend = createRemoteBackend({
    endpoint: { origin: `http://127.0.0.1:${upstreamPort}`, loopback: true }, serverToken: null,
    headersTimeoutMs: 1200, idleTimeoutMs: 1200, inferenceTimeoutMs: 1200,
    readinessBodyMaxBytes: 65536, readinessWholeMs: 2000,
  });
  instance = createInstance({ backend, token: null, authentication: 'none',
    host: '127.0.0.1', port: 0, inferenceTimeoutMs: 3000, idleTimeoutMs: 2500 });
  remotePort = (await bounded(instance.listen())).port;
  const send = path => bounded(new Promise((resolve, reject) => {
    client = http.request({ host: '127.0.0.1', port: remotePort, path, method: 'POST',
      agent: false, headers: { 'content-type': 'application/json' } }, response => {
      const chunks = [];
      response.on('data', chunk => chunks.push(chunk));
      response.once('error', reject);
      response.once('end', () => resolve({ status: response.statusCode, headers: response.headers,
        body: JSON.parse(Buffer.concat(chunks).toString()) }));
    });
    client.once('error', reject);
    client.end('{"model":"synthetic"}');
  }));
  return { send, requests };
}

for (const path of ['/openai/v1/responses', '/anthropic/v1/messages']) {
  test(`${path} reports a closed upgrade as upstream failure, not timeout`, { timeout: 6000 }, async context => {
    const state = await fixture(context);
    const response = await state.send(path);
    context.diagnostic(JSON.stringify({ path, status: response.status, code: response.body.error?.code }));
    assert.deepEqual(state.requests, [`POST ${path} HTTP/1.1`]);
    assert.equal(response.status, 502);
    assert.equal(response.body.error.code, 'upstream_error');
    assert.equal(response.headers['x-should-retry'], 'false');
    if (path.startsWith('/anthropic/')) assert.equal(response.body.type, 'error');
  });
}
