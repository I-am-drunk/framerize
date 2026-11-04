import assert from 'node:assert/strict';
import { createServer, request, type IncomingMessage } from 'node:http';
import { once } from 'node:events';
import type { AddressInfo } from 'node:net';
import { setTimeout as delay } from 'node:timers/promises';
import { isDeepStrictEqual } from 'node:util';

export function transportResources(): string[] {
  return process.getActiveResourcesInfo().filter(name => /TCP|Timeout/.test(name)).sort();
}

export async function assertTransportClosed(expected: readonly string[], timeoutMs = 1000): Promise<void> {
  const deadline = performance.now() + timeoutMs;
  let actual = transportResources();
  while (!isDeepStrictEqual(actual, expected) && performance.now() < deadline) {
    await delay(10);
    actual = transportResources();
  }
  assert.deepEqual(actual, expected, 'Transport left TCP or timer resources');
}

export const upstreamToken = 'synthetic-upstream-credential';
export const toolRequest = { model: 'fixture', stream: true, messages: [
  { role: 'user', content: 'Use the fixture tool' }], tools: [{ type: 'function',
    function: { name: 'fixture_tool', parameters: { type: 'object' } } }] };
export const toolResult = { model: 'fixture', stream: true, messages: [
  { role: 'assistant', tool_calls: [{ id: 'call_fixture', type: 'function',
    function: { name: 'fixture_tool', arguments: '{}' } }] },
  { role: 'tool', tool_call_id: 'call_fixture', content: 'synthetic-result' }] };
const first = Buffer.from('data: {"choices":[{"delta":{"tool_calls":[{"id":"call_fixture","type":"function","function":{"name":"fixture_tool","arguments":"{}"}}]}}]}\n\n');
const last = Buffer.from('data: [DONE]\n\n');
const result = Buffer.from('data: {"choices":[{"delta":{"content":"résultat"}}]}\n\ndata: [DONE]\n\n');
const delayedRequest = { fixture: 'delayed-json', stream: false };

export interface Reply { status: number; body: Buffer; }
export interface Observation { path: string; body: Buffer; authorization: string | undefined; }

export function exchange(base: string, path: string, token: string, options: {
  body?: object; onChunk?: () => void; abort?: boolean; timeoutMs?: number;
} = {}): Promise<Reply> {
  return new Promise((resolve, reject) => {
    const client = request(new URL(path, base), { agent: false,
      method: options.body ? 'POST' : 'GET',
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
    }, (response: IncomingMessage) => {
      const chunks: Buffer[] = [];
      response.on('data', (chunk: Buffer) => {
        chunks.push(chunk);
        options.onChunk?.();
        if (options.abort) client.destroy(new Error('Fixture client cancelled'));
      });
      response.on('error', reject);
      response.on('end', () => resolve({ status: response.statusCode!, body: Buffer.concat(chunks) }));
    });
    const deadline = setTimeout(() => client.destroy(new Error('Fixture request timed out')), options.timeoutMs ?? 5000);
    client.once('close', () => clearTimeout(deadline));
    client.on('error', reject);
    client.end(options.body ? JSON.stringify(options.body) : undefined);
  });
}

export async function createScenario() {
  const state = { ready: true, cancelled: 0, release: () => {},
    requests: [] as Observation[], failures: [] as unknown[] };
  const upstream = createServer(async (incoming, response) => {
    try {
      const chunks: Buffer[] = [];
      for await (const chunk of incoming) chunks.push(Buffer.from(chunk));
      const body = Buffer.concat(chunks);
      assert.ok(body.length < 8192);
      state.requests.push({ path: incoming.url!, body, authorization: incoming.headers.authorization });
      assert.equal(incoming.headers.authorization, `Bearer ${upstreamToken}`);
      if (incoming.url === '/v1/models') {
        response.writeHead(state.ready ? 200 : 503, { 'content-type': 'application/json' });
        response.end('{"data":[]}');
        return;
      }
      const payload: unknown = JSON.parse(body.toString());
      if (JSON.stringify(payload) === JSON.stringify(delayedRequest)) {
        assert.ok(['/v1/responses', '/v1/chat/completions'].includes(incoming.url!));
        const timer = setTimeout(() => {
          response.writeHead(200, { 'content-type': 'application/json' });
          response.end('{"done":true}');
        }, 16_000);
        response.once('close', () => clearTimeout(timer));
        return;
      }
      assert.equal(incoming.url, '/v1/chat/completions');
      response.writeHead(200, { 'content-type': 'text/event-stream' });
      if (JSON.stringify(payload) === JSON.stringify(toolRequest)) {
        const released = new Promise<void>((resolve) => { state.release = resolve; });
        response.write(first);
        await released;
        response.end(last);
      } else if (JSON.stringify(payload) === JSON.stringify(toolResult)) {
        response.end(result);
      } else {
        assert.deepEqual(payload, { fixture: 'disconnect' });
        response.once('close', () => { state.cancelled += 1; upstream.emit('cancelled'); });
        response.write(first);
      }
    } catch (error) { state.failures.push(error); response.destroy(); }
  });
  upstream.listen(0, '127.0.0.1');
  await once(upstream, 'listening');
  const url = `http://127.0.0.1:${(upstream.address() as AddressInfo).port}`;
  return { state, upstream, url, async close() {
    state.release();
    upstream.closeAllConnections();
    await new Promise<void>((resolve) => upstream.close(() => resolve()));
  } };
}

export async function exerciseDelayedJson(targets: readonly { url: string; token: string }[],
  scenario: Awaited<ReturnType<typeof createScenario>>) {
  const { state } = scenario;
  const before = state.requests.length;
  const cancelled = state.cancelled;
  const completed = await Promise.all(targets.flatMap(({ url, token }) =>
    ['/v1/responses', '/v1/chat/completions'].map(async path => {
      const response = await exchange(url, path, token, { body: delayedRequest, timeoutMs: 25_000 });
      assert.equal(response.status, 200);
      assert.equal(response.body.toString(), '{"done":true}');
      return response;
    })));
  const requests = state.requests.slice(before).filter(entry => entry.path !== '/v1/models');
  assert.equal(requests.length, targets.length * 2);
  assert.ok(requests.every(entry => entry.body.equals(Buffer.from(JSON.stringify(delayedRequest)))));
  assert.equal(state.cancelled, cancelled);
  assert.deepEqual(state.failures, []);
  return completed.length;
}

export async function exerciseFlow(base: string, token: string,
  scenario: Awaited<ReturnType<typeof createScenario>>) {
  const { state, upstream } = scenario;
  const inferenceCount = () => state.requests.filter((entry) => entry.path !== '/v1/models').length;
  const beforeWrongToken = state.requests.length;
  assert.equal((await exchange(base, '/v1/chat/completions', 'wrong-fixture-token',
    { body: toolRequest })).status, 401);
  assert.equal(state.requests.length, beforeWrongToken);
  const ready = await exchange(base, '/ready', token);
  assert.equal(ready.status, 200);
  assert.equal(JSON.parse(ready.body.toString()).status, 'ready');
  let chunksBeforeRelease = 0;
  const toolCall = await exchange(base, '/v1/chat/completions', token, { body: toolRequest,
    onChunk() { chunksBeforeRelease += 1; state.release(); } });
  assert.ok(chunksBeforeRelease > 0);
  assert.deepEqual(toolCall, { status: 200, body: Buffer.concat([first, last]) });
  assert.deepEqual(await exchange(base, '/v1/chat/completions', token, { body: toolResult }),
    { status: 200, body: result });
  assert.equal(inferenceCount(), 2);
  assert.deepEqual(state.requests.filter((entry) => entry.path !== '/v1/models')
    .map((entry) => entry.body), [toolRequest, toolResult].map((body) => Buffer.from(JSON.stringify(body))));
  state.ready = false;
  const unavailable = await exchange(base, '/ready', token);
  assert.equal(unavailable.status, 503);
  assert.equal(JSON.parse(unavailable.body.toString()).reason, 'upstream_unavailable');
  assert.equal(inferenceCount(), 2);
  state.ready = true;
  const controller = new AbortController();
  const deadline = setTimeout(() => controller.abort(), 5000);
  try {
    const cancelled = once(upstream, 'cancelled', { signal: controller.signal });
    await Promise.all([cancelled, assert.rejects(exchange(base, '/v1/chat/completions', token,
      { body: { fixture: 'disconnect' }, abort: true }), /Fixture client cancelled|aborted/)]);
  } finally { clearTimeout(deadline); }
  assert.equal(state.cancelled, 1);
  assert.equal(inferenceCount(), 3);
  assert.deepEqual(state.failures, []);
  return { inferenceRequests: inferenceCount(), toolRoundTrips: 1, cancellations: state.cancelled };
}
