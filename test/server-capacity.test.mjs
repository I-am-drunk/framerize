import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer as httpServer, request } from 'node:http';
import { once, getEventListeners } from 'node:events';
import { access } from 'node:fs/promises';
import { createServer } from '../framerize-server/dist/server.js';
import { createFramerBackend } from '../framerize-server/dist/framer-backend.js';
import { createMinter, resolveOptions } from '../framerize-server/dist/castle.js';
import { createRemoteBackend } from '../framerize-remote/dist/index.js';

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

function post(origin, token, path) {
  return new Promise((resolve, reject) => {
    const outgoing = request(new URL(path, origin), { method: 'POST', agent: false,
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' } }, response => {
      const chunks = [];
      response.on('data', chunk => chunks.push(chunk));
      response.on('error', reject);
      response.on('end', () => resolve({ status: response.statusCode,
        body: JSON.parse(Buffer.concat(chunks).toString()) }));
    });
    outgoing.on('error', reject);
    const input = path === '/v1/responses' ? { input: 'hello' }
      : { messages: [{ role: 'user', content: 'hello' }] };
    outgoing.end(JSON.stringify({ model: 'synthetic-model', ...input, stream: false }));
  });
}

for (const path of ['/v1/responses', '/v1/chat/completions']) for (const viaRemote of [false, true]) {
  test(`busy mint is not a credential failure: ${path}, remote=${viaRemote}`, { timeout: 5000 }, async context => {
    const entered = deferred();
    const release = deferred();
    const directories = [];
    const signals = [];
    const deviceIds = [];
    let collections = 0;
    let inferences = 0;
    let authClosed = 0;
    const minter = createMinter(resolveOptions({ timeoutMs: 3000, budgetMs: 2000 }), {
      async collect({ directory }) {
        directories.push(directory);
        if (++collections === 1) { entered.resolve(); await release.promise; }
        return { text: '{}', cuid: null, traceEvents: 0, replayed: false };
      },
      async encode() { return 'synthetic-castle'; },
    });
    const upstream = httpServer((incoming, response) => {
      inferences += 1; incoming.resume(); response.end('{"ok":true}');
    });
    let server;
    let remote;
    context.after(async () => {
      release.resolve();
      await remote?.close();
      await server?.close();
      await minter.close();
      upstream.closeAllConnections();
      await new Promise(resolve => upstream.close(resolve));
    });
    upstream.listen(0, '127.0.0.1');
    await once(upstream, 'listening');
    const backend = createFramerBackend({ projectId: 'synthetic-project',
      castleDeviceId: 'synthetic-capacity-device',
      origin: `http://127.0.0.1:${upstream.address().port}`,
      auth: { async accessToken() { return 'synthetic-upstream-token'; },
        async close() { authClosed += 1; } },
      castle: { mint(input) { signals.push(input.signal); deviceIds.push(input.cuid); return minter.mint(input); },
        close: () => minter.close() },
    });
    server = await createServer({ host: '127.0.0.1', port: 0, serverToken: 'server-token' },
      { provider: backend });
    let origin = `http://127.0.0.1:${(await server.listen()).port}`;
    if (viaRemote) {
      const forwarding = createRemoteBackend({ endpoint: { origin, loopback: true },
        serverToken: 'server-token', headersTimeoutMs: 2000, idleTimeoutMs: 2000,
        readinessWholeMs: 2000, readinessBodyMaxBytes: 65536 });
      remote = await createServer({ host: '127.0.0.1', port: 0, serverToken: 'local-token' },
        { provider: forwarding });
      origin = `http://127.0.0.1:${(await remote.listen()).port}`;
    }
    const token = viaRemote ? 'local-token' : 'server-token';
    const first = post(origin, token, path);
    await entered.promise;
    const refused = await post(origin, token, path);
    assert.equal(refused.status, 503);
    assert.equal(refused.body.error.code, 'provider_busy');
    assert.equal(refused.body.error.message, 'Provider is busy');
    assert.equal(inferences, 0);
    assert.equal(minter.stats().busy, true);
    release.resolve();
    assert.deepEqual(await first, { status: 200, body: { ok: true } });
    assert.deepEqual(await post(origin, token, path), { status: 200, body: { ok: true } });
    assert.equal(inferences, 2);
    assert.equal(collections, 2);
    assert.equal(minter.stats().failures, 1);
    assert.equal(minter.stats().busy, false);
    assert.deepEqual(deviceIds, Array(3).fill('synthetic-capacity-device'));
    for (const signal of signals) assert.equal(getEventListeners(signal, 'abort').length, 0);
    for (const directory of directories) await assert.rejects(access(directory), { code: 'ENOENT' });
    await remote?.close();
    await server.close();
    assert.equal(authClosed, 1);
  });
}
