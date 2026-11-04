import test from 'node:test';
import assert from 'node:assert/strict';
import { Agent, createServer as httpServer } from 'node:http';
import { once } from 'node:events';
import { createServer } from '../framerize-server/dist/server.js';
import { createRemoteBackend } from '../framerize-remote/dist/index.js';
import { post, protocols } from './custom-routes-fixture.mjs';

async function remoteFixture(context, respond) {
  const calls = [];
  const server = httpServer((incoming, response) => {
    calls.push({ path: incoming.url, authorization: incoming.headers.authorization });
    incoming.resume();
    respond(response);
  });
  const agent = new Agent({ keepAlive: false, proxyEnv: {} });
  let remote;
  context.after(async () => {
    agent.destroy();
    await remote?.close();
    await new Promise(resolve => server.close(resolve));
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const backend = createRemoteBackend({ endpoint: {
    origin: `http://127.0.0.1:${server.address().port}`, loopback: true },
  serverToken: 'synthetic-server', headersTimeoutMs: 1000, idleTimeoutMs: 1000,
  inferenceTimeoutMs: 2000, readinessWholeMs: 1000, readinessBodyMaxBytes: 65536 });
  remote = await createServer({ host: '127.0.0.1', port: 0, serverToken: 'synthetic-client' },
    { provider: backend });
  const origin = `http://127.0.0.1:${(await remote.listen()).port}`;
  return { calls, post: protocol => post(origin, protocol.path, protocol.body, agent) };
}

for (const protocol of protocols) {
  for (const mode of ['length', 'chunked', 'oversized']) {
    test(`Remote buffers ${protocol.name} and rejects ${mode} failure`, async context => {
      const setup = await remoteFixture(context, response => {
        response.setHeader('content-type', 'application/json');
        if (mode === 'length') response.setHeader('content-length', 50);
        if (mode === 'chunked') {
          const socket = response.socket;
          response.write('{}', () => socket.end());
        } else response.end(mode === 'oversized' ? Buffer.alloc(16 * 1024 * 1024 + 1, 32) : '{}');
      });
      const result = await setup.post(protocol);
      assert.equal(result.status, 502);
      assert.equal(result.headers['x-should-retry'], 'false');
      assert.deepEqual(setup.calls, [{ path: protocol.path, authorization: 'Bearer synthetic-server' }]);
    });
  }
  test(`Remote preserves opaque ${protocol.name} replies without parsing JSON`, async context => {
    const payload = Buffer.from('opaque custom reply');
    const setup = await remoteFixture(context, response => response.end(payload));
    const result = await setup.post(protocol);
    assert.equal(result.status, 200);
    assert.deepEqual(result.body, payload);
  });
  test(`Remote never falls back when an older server rejects ${protocol.name}`, async context => {
    const setup = await remoteFixture(context, response => { response.statusCode = 404; response.end('{}'); });
    assert.equal((await setup.post(protocol)).status, 404);
    assert.equal(setup.calls.length, 1);
    assert.equal(setup.calls[0].path, protocol.path);
  });
}
