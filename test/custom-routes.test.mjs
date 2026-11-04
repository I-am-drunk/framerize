import test from 'node:test';
import assert from 'node:assert/strict';
import { fixture, protocols, reply } from './custom-routes-fixture.mjs';

for (const protocol of protocols) for (const viaRemote of [false, true]) {
  const name = `${protocol.name}, remote=${viaRemote}`;
  if (protocol.name === 'responses') for (const argumentText of ['', '{', '{"query":']) {
    test(`unfinished function arguments survive a limit: ${JSON.stringify(argumentText)}, ${name}`,
      async context => {
        const value = reply(protocol, { status: 'incomplete',
          incomplete_details: { reason: 'max_output_tokens' }, output: [
            { type: 'function_call', call_id: 'call_limit', name: 'lookup',
              status: 'incomplete', arguments: argumentText },
          ] });
        const setup = await fixture(context, protocol, viaRemote, { value });
        const result = await setup.post();
        assert.equal(result.status, 200);
        assert.deepEqual(result.body, setup.payload);
        assert.equal(setup.observed.length, 1);
        assert.equal(setup.counts.mints, 1);
      });
  }
  test(`Castle device identity remains server-owned: ${name}`, async context => {
    const castleDeviceId = 'synthetic-server-device';
    const setup = await fixture(context, protocol, viaRemote, { backend: { castleDeviceId } });
    const result = await setup.post(protocol.body, { 'x-castle-device-id': 'untrusted-client-device' });
    assert.equal(result.status, 200);
    assert.deepEqual(setup.deviceIds, [castleDeviceId]);
    assert.equal(setup.observed.length, 1);
    assert.equal(setup.observed[0].headers['x-castle-device-id'], undefined);
    assert.equal(result.headers['x-castle-device-id'], undefined);
    assert.equal(result.body.includes(castleDeviceId), false);
    assert.deepEqual(setup.observed[0].body.request, protocol.body);
  });
  if (protocol.name === 'responses') for (const type of ['function_call', 'custom_tool_call']) {
    test(`output-limited ${type} is preserved: ${name}`, async context => {
      const output = [{ type, call_id: 'call_limit', name: 'lookup', status: 'incomplete',
        ...(type === 'function_call' ? { arguments: '{}' } : { input: 'partial text' }) }];
      const value = reply(protocol, { status: 'incomplete', output,
        incomplete_details: { reason: 'max_output_tokens' } });
      const setup = await fixture(context, protocol, viaRemote, { value });
      const result = await setup.post();
      assert.equal(result.status, 200);
      assert.deepEqual(result.body, setup.payload);
      assert.equal(setup.observed.length, 1);
      assert.equal(setup.counts.mints, 1);
    });
  }
  if (protocol.name === 'responses') {
    test(`background jobs cannot start on a completed-response route: ${name}`, async context => {
      const setup = await fixture(context, protocol, viaRemote);
      const result = await setup.post({ ...protocol.body, background: true });
      assert.equal(result.status, 400);
      assert.equal(setup.counts.auth, 0);
      assert.equal(setup.counts.mints, 0);
      assert.equal(setup.observed.length, 0);
    });
  }
  test(`disconnect cancels the active completed reply: ${name}`, { timeout: 4000 }, async context => {
    const setup = await fixture(context, protocol, viaRemote, { mode: 'stall' });
    const controller = new AbortController();
    const pending = assert.rejects(setup.post(protocol.body, {}, controller.signal), { name: 'AbortError' });
    await setup.entered;
    controller.abort();
    await pending;
    await setup.disconnected;
    assert.equal(setup.signals[0].aborted, true);
    assert.equal(setup.observed.length, 1);
  });
  for (const limit of ['backend', 'server', ...(viaRemote ? ['remote'] : [])]) {
    test(`total ${limit} deadline covers the completed body: ${name}`, async context => {
      const setup = await fixture(context, protocol, viaRemote, {
        mode: 'stall', [limit]: { inferenceTimeoutMs: 100 },
      });
      const result = await setup.post();
      assert.equal(result.status, 504);
      assert.equal(result.headers['x-should-retry'], 'false');
      assert.equal(JSON.parse(result.body).error.code, 'headers_timeout');
      await setup.disconnected;
      assert.equal(setup.observed.length, 1);
    });
  }
  test(`tool call and rich tool result survive a round trip: ${name}`, async context => {
    const call = protocol.name === 'responses'
      ? { type: 'function_call', call_id: 'synthetic-call', name: 'lookup', arguments: '{"query":"snow"}' }
      : { type: 'tool_use', id: 'synthetic-call', name: 'lookup', input: { query: 'snow' } };
    const first = reply(protocol, protocol.name === 'responses'
      ? { output: [call] } : { content: [call], stop_reason: 'tool_use' });
    const setup = await fixture(context, protocol, viaRemote, {
      respond(response, count) { response.setHeader('content-type', 'application/json');
        response.end(JSON.stringify(count === 1 ? first : reply(protocol))); },
    });
    const result = await setup.post();
    assert.deepEqual(JSON.parse(result.body), first);
    const rich = [{ type: protocol.name === 'responses' ? 'input_text' : 'text', text: 'snow' }];
    const body = protocol.name === 'responses'
      ? { ...protocol.body, input: [call,
        { type: 'function_call_output', call_id: call.call_id, output: rich }] }
      : { ...protocol.body, messages: [...protocol.body.messages,
        { role: 'assistant', content: [call] }, { role: 'user', content: [
          { type: 'tool_result', tool_use_id: call.id, content: rich }] }] };
    const final = await setup.post(body);
    assert.equal(final.status, 200);
    assert.deepEqual(JSON.parse(final.body), reply(protocol));
    assert.deepEqual(setup.observed.map(item => item.body.request), [protocol.body, body]);
    assert.equal(setup.counts.mints, 2);
    assert.equal(setup.counts.auth, 2);
  });
  for (const failure of [
    { name: 'invalid JSON', payload: '{"SYNTHETIC_SECRET":' },
    { name: 'nonterminal JSON', value: reply(protocol, { status: 'in_progress', stop_reason: null }) },
    { name: 'short Content-Length', mode: 'length' },
    { name: 'unfinished chunked response', mode: 'chunked' },
    { name: 'SSE instead of JSON', headers: { 'content-type': 'text/event-stream' } },
    { name: 'compressed response', headers: { 'content-encoding': 'gzip' } },
    { name: 'oversized response', payload: ' '.repeat(16 * 1024 * 1024 + 1) },
  ]) {
    test(`custom reply rejects ${failure.name} before sending success: ${name}`, async context => {
      const setup = await fixture(context, protocol, viaRemote, failure);
      const result = await setup.post();
      assert.equal(result.status, 502);
      assert.equal(result.headers['x-should-retry'], 'false');
      assert.equal(result.body.includes('SYNTHETIC_SECRET'), false);
      if (protocol.name === 'messages') assert.equal(JSON.parse(result.body).type, 'error');
      assert.equal(setup.observed.length, 1);
      assert.equal(setup.counts.mints, 1);
    });
  }
  test(`custom HTTP refusal is sanitized without enabling replay: ${name}`, async context => {
    const setup = await fixture(context, protocol, viaRemote, { status: 429,
      payload: 'SYNTHETIC_SECRET', headers: { 'retry-after': '10', 'x-should-retry': 'true' } });
    const result = await setup.post();
    assert.equal(result.status, 429);
    assert.equal(result.headers['retry-after'], '10');
    assert.equal(result.headers['x-should-retry'], 'false');
    assert.equal(result.body.includes('SYNTHETIC_SECRET'), false);
    assert.equal(setup.observed.length, 1);
  });
  test(`completed custom reply preserves bytes and credentials: ${name}`, async context => {
    const setup = await fixture(context, protocol, viaRemote);
    const body = { ...protocol.body, stream: undefined, temperature: 0.2,
      metadata: { retained: 'value' }, projectId: 'must-not-override-envelope' };
    const result = await setup.post(body, { 'anthropic-version': '2026-10-21',
      'anthropic-beta': 'synthetic-beta', 'openai-beta': 'synthetic-openai',
      cookie: 'synthetic-private-cookie', 'x-api-key': 'synthetic-private-key' });
    assert.equal(result.status, 200);
    assert.deepEqual(result.body, setup.payload);
    assert.equal(setup.observed.length, 1);
    const upstream = setup.observed[0];
    assert.equal(upstream.path, protocol.upstream);
    assert.deepEqual(upstream.body.request, { ...body, stream: false });
    assert.equal(upstream.body.projectId, 'synthetic-project');
    assert.equal(upstream.headers.authorization, 'Bearer synthetic-upstream');
    assert.equal(upstream.headers['accept-encoding'], 'identity');
    assert.equal(upstream.headers['anthropic-version'], '2026-10-21');
    assert.equal(upstream.headers.cookie, undefined);
    assert.equal(upstream.headers['x-api-key'], undefined);
    assert.equal(setup.counts.auth, 1);
    assert.equal(setup.counts.mints, 1);
    assert.deepEqual(setup.serverCalls, [{ path: protocol.path,
      authorization: viaRemote ? 'Bearer synthetic-server' : 'Bearer synthetic-client' }]);
  });
  test(`invalid custom requests cannot access provider credentials: ${name}`, async context => {
    const setup = await fixture(context, protocol, viaRemote);
    const unauthorized = await setup.post(protocol.body, { authorization: 'Bearer wrong' });
    assert.equal(unauthorized.status, 401);
    for (const body of ['{"invalid":',
      { ...protocol.body, stream: null },
      ...(protocol.name === 'responses' ? [{ ...protocol.body, background: true }] : []),
      { ...protocol.body, model: '' }, protocol.name === 'responses'
        ? { ...protocol.body, input: null } : { ...protocol.body, max_tokens: 0 }]) {
      const result = await setup.post(body);
      assert.equal(result.status, 400);
      assert.equal(JSON.parse(result.body).error.code,
        typeof body === 'string' ? 'invalid_json' : 'invalid_request_body');
      if (protocol.name === 'messages') assert.equal(JSON.parse(result.body).type, 'error');
    }
    assert.equal(setup.counts.auth, 0);
    assert.equal(setup.counts.mints, 0);
    assert.equal(setup.observed.length, 0);
    assert.ok(setup.serverCalls.every(call => call.path === protocol.path));
  });
  for (const outcome of ['refusal', 'limit']) {
    test(`native ${outcome} remains a completed reply: ${name}`, async context => {
      const value = protocol.name === 'responses' ? reply(protocol, outcome === 'limit'
        ? { status: 'incomplete', incomplete_details: { reason: 'max_output_tokens' } }
        : { output: [{ type: 'message', id: 'msg', role: 'assistant',
          content: [{ type: 'refusal', refusal: 'Synthetic refusal.' }] }] })
        : reply(protocol, { stop_reason: outcome === 'limit' ? 'max_tokens' : 'refusal' });
      const setup = await fixture(context, protocol, viaRemote, { value });
      const result = await setup.post();
      assert.equal(result.status, 200);
      assert.deepEqual(result.body, setup.payload);
      assert.equal(setup.observed.length, 1);
    });
  }
}
