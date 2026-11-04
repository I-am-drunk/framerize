import assert from 'node:assert/strict';
import { once } from 'node:events';
import { createServer } from 'node:http';
import { isDeepStrictEqual } from 'node:util';
import { exchange } from './transport-scenario.js';

function scenarioCase(name: 'responses' | 'messages') {
  const responses = name === 'responses';
  const firstInput = { role: 'user', content: 'Use the fixture tool' };
  const call = responses
    ? { type: 'function_call', call_id: 'call_fixture', name: 'lookup', arguments: '{}' }
    : { type: 'tool_use', id: 'call_fixture', name: 'lookup', input: {} };
  const request = (input: unknown) => responses
    ? { model: 'fixture', stream: false, input }
    : { model: 'fixture', stream: false, max_tokens: 64, messages: input };
  const initial = request([firstInput]);
  const followup = request(responses ? [firstInput, call,
    { type: 'function_call_output', call_id: 'call_fixture', output: 'synthetic-result' }]
    : [firstInput, { role: 'assistant', content: [call] }, { role: 'user', content: [
      { type: 'tool_result', tool_use_id: 'call_fixture', content: 'synthetic-result' }] }]);
  const reply = (tool: boolean) => responses
    ? { id: 'resp_fixture', object: 'response', created_at: 1, model: 'fixture', status: 'completed',
      usage: { input_tokens: 1, output_tokens: 2 },
      output: tool ? [call] : [{ type: 'message', id: 'msg_fixture', role: 'assistant',
        content: [{ type: 'output_text', text: 'Synthetic result.', annotations: [] }] }] }
    : { id: 'msg_fixture', type: 'message', role: 'assistant', model: 'fixture',
      usage: { input_tokens: 1, output_tokens: 2 },
      content: tool ? [call] : [{ type: 'text', text: 'Synthetic result.' }],
      stop_reason: tool ? 'tool_use' : 'end_turn', stop_sequence: null };
  return { name, path: responses ? '/openai/v1/responses' : '/anthropic/v1/messages',
    upstream: responses ? '/ai/v3/responses/' : '/ai/v3/messages/', initial, followup,
    first: Buffer.from(JSON.stringify(reply(true))), final: Buffer.from(JSON.stringify(reply(false))),
    invalid: request([{ role: 'user', content: 'bad-reply' }]),
    cancel: request([{ role: 'user', content: 'cancel' }]) };
}

export const customCases = [scenarioCase('responses'), scenarioCase('messages')];

export async function createCustomScenario() {
  const state = { auth: 0, inference: 0, cancelled: 0, failures: [] as unknown[] };
  const gateway = createServer(async (incoming, response) => {
    try {
      const chunks: Buffer[] = [];
      for await (const chunk of incoming) chunks.push(Buffer.from(chunk));
      if (incoming.url === '/auth/ai/access-token') {
        assert.equal(incoming.headers.cookie, 'session=synthetic-account');
        state.auth += 1;
        response.setHeader('content-type', 'application/json');
        response.end(JSON.stringify({ accessToken: 'synthetic-access',
          expiresAt: new Date(Date.now() + 600_000).toISOString() }));
        return;
      }
      state.inference += 1;
      const entry = customCases.find(value => value.upstream === incoming.url);
      assert.ok(entry, 'Custom route must use its exact upstream path');
      assert.equal(incoming.headers.authorization, 'Bearer synthetic-access');
      assert.equal(incoming.headers.cookie, undefined);
      const payload = JSON.parse(Buffer.concat(chunks).toString());
      assert.equal(payload.projectId, 'selected-project');
      assert.match(payload.castleToken, /^cNz8rPYH\|[A-Za-z0-9+/]+={0,2}$/);
      response.setHeader('content-type', 'application/json');
      if (isDeepStrictEqual(payload.request, entry.initial)) response.end(entry.first);
      else if (isDeepStrictEqual(payload.request, entry.followup)) response.end(entry.final);
      else if (isDeepStrictEqual(payload.request, entry.invalid)) response.end('{"unfinished":');
      else if (isDeepStrictEqual(payload.request, entry.cancel)) {
        response.once('close', () => { state.cancelled += 1; gateway.emit('inference-closed'); });
        response.write('{');
        gateway.emit('inference-waiting');
      } else assert.fail('Unexpected custom request');
    } catch (error) { state.failures.push(error); response.destroy(); }
  });
  gateway.listen(0, '127.0.0.1');
  await once(gateway, 'listening');
  const address = gateway.address();
  assert.ok(address && typeof address !== 'string');
  return { state, gateway, url: `http://127.0.0.1:${address.port}`, async close() {
    gateway.closeAllConnections();
    await new Promise<void>(resolve => gateway.close(() => resolve()));
  } };
}

export async function exerciseCustomFlow(base: string, token: string,
  scenario: Awaited<ReturnType<typeof createCustomScenario>>) {
  for (const entry of customCases) {
    const before = scenario.state.inference;
    assert.equal((await exchange(base, entry.path, 'wrong', { body: entry.initial })).status, 401);
    assert.equal((await exchange(base, entry.path, token,
      { body: { ...entry.initial, stream: null } })).status, 400);
    assert.equal(scenario.state.inference, before);
    for (const [body, expected] of [[entry.initial, entry.first], [entry.followup, entry.final]] as const) {
      const result = await exchange(base, entry.path, token, { body, timeoutMs: 20_000 });
      assert.equal(result.status, 200);
      assert.deepEqual(result.body, expected);
    }
    for (const body of [entry.initial, entry.followup]) {
      const result = await exchange(base, entry.path, token, { body: { ...body, stream: true }, timeoutMs: 20_000 });
      assert.equal(result.status, 200);
      const terminal = entry.name === 'responses' ? 'response.completed' : 'message_stop';
      assert.ok(result.body.includes(`event: ${terminal}\n`));
      assert.equal(result.body.includes('event: error\n'), false);
      if (body === entry.initial) assert.ok(result.body.includes(entry.name === 'responses'
        ? 'response.function_call_arguments.delta' : 'input_json_delta'));
    }
    const invalid = await exchange(base, entry.path, token, { body: entry.invalid, timeoutMs: 20_000 });
    assert.equal(invalid.status, 502);
    assert.equal(JSON.parse(invalid.body.toString()).error.code, 'upstream_response_invalid');
    assert.equal(scenario.state.inference, before + 5);
    const controller = new AbortController();
    const arrived = once(scenario.gateway, 'inference-waiting', { signal: AbortSignal.timeout(20_000) });
    const closed = once(scenario.gateway, 'inference-closed', { signal: AbortSignal.timeout(20_000) });
    const pending = assert.rejects(exchange(base, entry.path, token,
      { body: entry.cancel, timeoutMs: 20_000, signal: controller.signal }), { name: 'AbortError' });
    await arrived;
    controller.abort();
    await pending;
    await closed;
    assert.equal(scenario.state.inference, before + 6);
  }
  assert.deepEqual(scenario.state.failures, []);
}
