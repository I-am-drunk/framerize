import assert from 'node:assert/strict';
import { request } from 'node:http';
import { getEventListeners } from 'node:events';
import test from 'node:test';
import type { InferenceBackend } from '../src/backend.js';
import { createInstance } from '../src/instance.js';
import { authorization, backend, comparable, deferred, exchange, token } from './http-fixtures.js';

const source = new URL('../../../framerize-server/dist/', import.meta.url);
const { createServer } = await import(new URL('server.js', source).href) as {
  createServer(config: object, dependencies: { provider: InferenceBackend }): Promise<{
    listen(): Promise<{ port: number }>; close(): Promise<void>;
  }>;
};
const { createRemoteBackend } = await import(new URL('../../../framerize-remote/dist/index.js', import.meta.url).href) as {
  createRemoteBackend(config: object): InferenceBackend;
};

function remoteBackend(upstream: string, remoteToken: string): InferenceBackend {
  return createRemoteBackend({ endpoint: { origin: upstream, loopback: true }, serverToken: remoteToken,
    headersTimeoutMs: 1000, idleTimeoutMs: 1000, readinessBodyMaxBytes: 65536, readinessWholeMs: 2000 });
}

test('client abort reaches the actual remote server and removes cancellation listeners',
  { timeout: 3000 }, async () => {
  const entered = deferred<AbortSignal>();
  const aborted = deferred<void>();
  const remote = await createServer({ host: '127.0.0.1', port: 0, serverToken: 'remote',
    accountsFile: '', upstream: '' }, { provider: backend({ async forward({ signal }) {
    signal.addEventListener('abort', () => aborted.resolve(), { once: true });
    entered.resolve(signal);
    return new Promise(() => {});
  } }) });
  const address = await remote.listen();
  const local = createInstance({ token, backend: remoteBackend(`http://127.0.0.1:${address.port}`, 'remote') });
  try {
    const { port } = await local.listen();
    const client = request({ host: '127.0.0.1', port, path: '/v1/responses',
      method: 'POST', headers: authorization, agent: false });
    client.on('error', () => {});
    client.end('{}');
    const signal = await entered.promise;
    client.destroy();
    await aborted.promise;
    assert.equal(signal.aborted, true);
    assert.equal(getEventListeners(signal, 'abort').length, 0);
  } finally { await Promise.all([local.close(), remote.close()]); }
});

test('local and remote credentials are separate and refusal never invokes inference', async () => {
  let calls = 0;
  const remote = await createServer({ host: '127.0.0.1', port: 0,
    serverToken: 'synthetic-remote', accountsFile: '', upstream: '' }, { provider: backend({
    async forward() { calls += 1; throw new Error('must not run'); },
  }) });
  const address = await remote.listen();
  const local = createInstance({ token,
    backend: remoteBackend(`http://127.0.0.1:${address.port}`, 'wrong-remote') });
  try {
    const { port } = await local.listen();
    const localRefusal = await exchange(port, '/v1/responses', { method: 'POST' });
    assert.equal(localRefusal.status, 401);
    assert.equal(JSON.parse(localRefusal.body.toString()).error.code, 'unauthorized');
    const remoteRefusal = await exchange(port, '/v1/responses', { method: 'POST', headers: authorization });
    assert.equal(remoteRefusal.status, 503);
    assert.equal(calls, 0);
    assert.doesNotMatch(remoteRefusal.body.toString(), /synthetic-remote|wrong-remote/);
    assert.equal((await exchange(port, '/ready', { headers: authorization })).status, 503);
  } finally { await Promise.all([local.close(), remote.close()]); }
});

test('local instance forwards exact tool-call bytes through the accepted server', async () => {
  const remoteToken = 'synthetic-remote-token';
  const requests: Buffer[] = [];
  const output = Buffer.from('data: {"type":"tool_call","name":"synthetic"}\n\ndata: [DONE]\n\n');
  let ready = true;
  const remote = await createServer({ host: '127.0.0.1', port: 0, serverToken: remoteToken,
    accountsFile: '', upstream: '' }, { provider: backend({
    async forward(request) {
      const chunks: Buffer[] = [];
      for await (const chunk of request.body) chunks.push(chunk);
      requests.push(Buffer.concat(chunks));
      assert.equal(request.headers.authorization, undefined);
      return { status: 200, headers: { 'content-type': 'text/event-stream' },
        body: (async function* () { yield output.subarray(0, 19); yield output.subarray(19); })() };
    },
    async ready() { return ready ? true : { ready: false, reason: 'upstream_unavailable' }; },
  }) });
  const address = await remote.listen();
  const local = createInstance({ token,
    backend: remoteBackend(`http://127.0.0.1:${address.port}`, remoteToken) });
  try {
    const { port } = await local.listen();
    assert.equal((await exchange(port, '/ready', { headers: authorization })).status, 200);
    ready = false;
    assert.equal((await exchange(port, '/ready', { headers: authorization })).status, 503);
    const refused = await exchange(port, '/v1/responses', { method: 'POST', headers: authorization }, Buffer.from('{}'));
    assert.equal(refused.status, 503);
    assert.equal(requests.length, 0);
    ready = true;
    const input = Buffer.from('{"model":"synthetic","tool_calls":[{"id":"fixture"}]}');
    for (const path of ['/v1/responses', '/v1/chat/completions']) {
      const forwarded = await exchange(port, path, { method: 'POST', headers: authorization }, input);
      const direct = await exchange(address.port, path, { method: 'POST',
        headers: { authorization: `Bearer ${remoteToken}` } }, input);
      assert.deepEqual(comparable(forwarded), comparable(direct));
      assert.deepEqual(forwarded.body, output);
    }
    assert.equal(requests.length, 4);
    for (const request of requests) assert.deepEqual(request, input);
  } finally { await Promise.all([local.close(), remote.close()]); }
});
