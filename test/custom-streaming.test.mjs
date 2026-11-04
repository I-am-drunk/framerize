import test from 'node:test';
import assert from 'node:assert/strict';
import { fixture, protocols, reply } from './custom-routes-fixture.mjs';

function events(bytes) {
  return bytes.toString().split('\n\n').flatMap(frame => {
    const data = frame.split('\n').find(line => line.startsWith('data: '));
    if (!data) return [];
    const event = JSON.parse(data.slice(6));
    assert.ok(frame.includes(`event: ${event.type}\n`));
    return [event];
  });
}

for (const protocol of protocols) for (const viaRemote of [false, true]) {
  const name = `${protocol.name}, remote=${viaRemote}`;
  test(`streamed tool call and result round trip: ${name}`, async context => {
    const tool = protocol.name === 'responses'
      ? { type: 'function_call', id: 'tool', call_id: 'call', name: 'lookup', arguments: '{"query":"snow"}' }
      : { type: 'tool_use', id: 'call', name: 'lookup', input: { query: 'snow' } };
    const first = reply(protocol, protocol.name === 'responses'
      ? { output: [tool] } : { content: [tool], stop_reason: 'tool_use' });
    const setup = await fixture(context, protocol, viaRemote, {
      respond(response, count) {
        response.setHeader('content-type', 'application/json');
        response.end(JSON.stringify(count === 1 ? first : reply(protocol)));
      },
    });
    const initial = await setup.post({ ...protocol.body, stream: true });
    const received = events(initial.body);
    assert.equal(received.some(item => item.type === 'response.function_call_arguments.delta'
      || item.delta?.type === 'input_json_delta'), true);
    const body = protocol.name === 'responses'
      ? { ...protocol.body, stream: true, input: [tool,
        { type: 'function_call_output', call_id: 'call', output: 'Found snow.' }] }
      : { ...protocol.body, stream: true, messages: [...protocol.body.messages,
        { role: 'assistant', content: [tool] }, { role: 'user', content: [
          { type: 'tool_result', tool_use_id: 'call', content: 'Found snow.' }] }] };
    const final = await setup.post(body);
    assert.equal(events(final.body).at(-1).type,
      protocol.name === 'responses' ? 'response.completed' : 'message_stop');
    assert.deepEqual(setup.observed.map(item => item.body.request), [protocol.body, { ...body, stream: false }]);
    assert.equal(setup.counts.mints, 2);
  });
  for (const owner of ['backend', 'server', ...(viaRemote ? ['remote'] : [])]) {
    test(`keepalives cannot extend the ${owner} deadline: ${name}`, async context => {
      let received = '';
      const setup = await fixture(context, protocol, viaRemote, {
        [owner]: { inferenceTimeoutMs: 150 }, respond() {},
        backend: { inferenceTimeoutMs: owner === 'backend' ? 150 : 2000, idleTimeoutMs: 80 },
        onChunk(chunk) { received += chunk; },
      });
      await assert.rejects(setup.post({ ...protocol.body, stream: true }));
      assert.ok(received.includes(': keepalive'));
      assert.equal(received.includes('response.completed'), false);
      assert.equal(received.includes('message_stop'), false);
      await setup.disconnected;
      assert.equal(setup.observed.length, 1);
    });
  }
  test(`bad local token starts no streaming work: ${name}`, async context => {
    const setup = await fixture(context, protocol, viaRemote);
    const result = await setup.post({ ...protocol.body, stream: true }, { authorization: 'Bearer wrong' });
    assert.equal(result.status, 401);
    assert.equal(setup.observed.length, 0);
    assert.equal(setup.counts.auth, 0);
    assert.equal(setup.counts.mints, 0);
  });
  for (const failure of [
    { name: 'invalid JSON', payload: '{"SYNTHETIC_SECRET":' },
    { name: 'nonterminal reply', value: reply(protocol, { status: 'in_progress', stop_reason: null }) },
    { name: 'HTTP refusal', status: 429, payload: 'SYNTHETIC_SECRET' },
    { name: 'oversized reply', payload: ' '.repeat(16 * 1024 * 1024 + 1) },
    { name: 'unfinished HTTP', mode: 'chunked' },
    { name: 'compressed reply', headers: { 'content-encoding': 'gzip' } },
  ]) test(`stream error without replay for ${failure.name}: ${name}`, async context => {
    const setup = await fixture(context, protocol, viaRemote, failure);
    const result = await setup.post({ ...protocol.body, stream: true });
    assert.equal(result.status, 200);
    assert.equal(result.headers['x-should-retry'], 'false');
    const received = events(result.body);
    assert.equal(received.length, 1);
    assert.equal(received[0].type, 'error');
    assert.equal(result.body.includes('SYNTHETIC_SECRET'), false);
    assert.equal(setup.observed.length, 1);
    assert.equal(setup.counts.mints, 1);
  });
  test(`disconnect cancels streaming work: ${name}`, async context => {
    const setup = await fixture(context, protocol, viaRemote, { mode: 'stall' });
    const controller = new AbortController();
    const result = assert.rejects(setup.post({ ...protocol.body, stream: true }, {}, controller.signal),
      { name: 'AbortError' });
    await setup.entered;
    controller.abort();
    await result;
    await setup.disconnected;
    assert.equal(setup.signals[0].aborted, true);
    assert.equal(setup.observed.length, 1);
  });
  test(`keepalives arrive before the completed upstream reply: ${name}`, async context => {
    let upstream;
    let heartbeats = 0;
    let completed = false;
    const setup = await fixture(context, protocol, viaRemote, {
      backend: { idleTimeoutMs: 60 },
      respond(response) { upstream = response; },
      onChunk(chunk) {
        heartbeats += chunk.toString().split(': keepalive').length - 1;
        if (heartbeats >= 3 && upstream && !completed) {
          completed = true;
          upstream.setHeader('content-type', 'application/json');
          upstream.end(JSON.stringify(reply(protocol)));
        }
      },
    });
    const result = await setup.post({ ...protocol.body, stream: true });
    assert.equal(completed, true);
    assert.ok(heartbeats >= 3);
    assert.equal(result.headers['x-should-retry'], 'false');
    assert.equal(setup.observed.length, 1);
    assert.equal(events(result.body).at(-1).type,
      protocol.name === 'responses' ? 'response.completed' : 'message_stop');
  });
  test(`client streams without streaming upstream: ${protocol.name}, remote=${viaRemote}`, async context => {
    const setup = await fixture(context, protocol, viaRemote);
    const result = await setup.post({ ...protocol.body, stream: true });
    assert.equal(result.status, 200);
    assert.match(result.headers['content-type'], /^text\/event-stream/);
    const received = events(result.body);
    assert.equal(received.at(-1).type,
      protocol.name === 'responses' ? 'response.completed' : 'message_stop');
    assert.equal(setup.observed.length, 1);
    assert.equal(setup.observed[0].path, protocol.upstream);
    assert.deepEqual(setup.observed[0].body.request, protocol.body);
    assert.equal(setup.counts.mints, 1);
    assert.equal(setup.counts.auth, 1);
  });
}
