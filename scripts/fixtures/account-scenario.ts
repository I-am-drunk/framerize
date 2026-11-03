import assert from 'node:assert/strict';
import { once } from 'node:events';
import { createServer } from 'node:http';
import { exchange } from './transport-scenario.js';

const output = 'data: {"choices":[{"delta":{"content":"fixture"}}]}\n\ndata: [DONE]\n\n';
const request = { model: 'fixture', stream: true, messages: [
  { role: 'assistant', tool_calls: [{ id: 'call-1', type: 'function', function: { name: 'lookup', arguments: '{}' } }] },
  { role: 'tool', tool_call_id: 'call-1', content: 'synthetic-result' },
] };

export async function createAccountScenario() {
  let cancelled!: () => void;
  const cancellation = new Promise<void>(resolve => { cancelled = resolve; });
  const state = { auth: 0, inference: 0, cancelled: 0, failures: [] as unknown[] };
  const gateway = createServer(async (incoming, response) => {
    try {
      const chunks: Buffer[] = [];
      for await (const chunk of incoming) chunks.push(Buffer.from(chunk));
      const bytes = Buffer.concat(chunks);
      assert.ok(bytes.length < 65536);
      if (incoming.url === '/auth/ai/access-token') {
        assert.equal(incoming.headers.cookie, 'session=synthetic-account');
        state.auth += 1;
        response.setHeader('content-type', 'application/json');
        response.end(JSON.stringify({ accessToken: 'synthetic-access',
          expiresAt: new Date(Date.now() + 600_000).toISOString() }));
        return;
      }
      state.inference += 1;
      assert.equal(incoming.url, '/ai/v3/chat/');
      assert.equal(incoming.headers.authorization, 'Bearer synthetic-access');
      assert.equal(incoming.headers.cookie, undefined);
      const payload = JSON.parse(bytes.toString());
      assert.equal(payload.projectId, 'selected-project');
      assert.equal(payload.featureCategory, 'agents');
      assert.match(payload.castleToken, /^cNz8rPYH\|[A-Za-z0-9+/]+={0,2}$/);
      response.writeHead(200, { 'content-type': 'text/event-stream' });
      if (payload.request.messages[0]?.content === 'cancel') {
        response.once('close', () => { state.cancelled += 1; cancelled(); });
        response.write('data: waiting\n\n');
      } else {
        assert.deepEqual(payload.request, request);
        response.end(output);
      }
    } catch (error) {
      state.failures.push(error); response.statusCode = 500; response.end('{}');
    }
  });
  gateway.listen(0, '127.0.0.1'); await once(gateway, 'listening');
  const address = gateway.address(); assert.ok(address && typeof address !== 'string');
  return { url: `http://127.0.0.1:${address.port}`, state, cancellation,
    async close() {
      gateway.closeAllConnections();
      await new Promise<void>(resolve => gateway.close(() => resolve()));
    },
  };
}

export async function exerciseAccountFlow(base: string, token: string,
  scenario: Awaited<ReturnType<typeof createAccountScenario>>) {
  assert.equal((await exchange(base, '/ready', 'wrong')).status, 401);
  assert.equal(scenario.state.auth, 0);
  assert.equal((await exchange(base, '/ready', token)).status, 200);
  assert.equal(scenario.state.auth, 1); assert.equal(scenario.state.inference, 0);
  const response = await exchange(base, '/v1/chat/completions', token, { body: request, timeoutMs: 20_000 });
  assert.equal(response.status, 200, response.body.toString());
  assert.equal(response.body.toString(), output);
  await assert.rejects(exchange(base, '/v1/chat/completions', token, { timeoutMs: 20_000, abort: true,
    body: { model: 'fixture', stream: true, messages: [{ role: 'user', content: 'cancel' }] },
  }));
  await waitForCancellation(scenario.cancellation);
  assert.deepEqual(scenario.state.failures, []);
  assert.equal(scenario.state.inference, 2); assert.equal(scenario.state.cancelled, 1);
  return { accountSelected: true, builtInMints: 2, streamed: true, cancelled: true };
}

async function waitForCancellation(promise: Promise<void>): Promise<void> {
  let timer: NodeJS.Timeout | undefined;
  try {
    await Promise.race([promise, new Promise<never>((_resolve, reject) => {
      timer = setTimeout(() => reject(new Error('Account upstream did not close')), 5000);
    })]);
  } finally { clearTimeout(timer); }
}
