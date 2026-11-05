import test from 'node:test';
import assert from 'node:assert/strict';
import { fixture, protocols, reply } from './custom-routes-fixture.mjs';

for (const viaRemote of [false, true]) {
  test(`Messages citation events preserve order and block identity, remote=${viaRemote}`, async context => {
    const protocol = protocols.find(value => value.name === 'messages');
    const citations = [
      { type: 'char_location', cited_text: 'Snow', document_index: 0,
        document_title: 'Weather', file_id: null, start_char_index: 0, end_char_index: 4 },
      { type: 'web_search_result_location', cited_text: 'Snow',
        title: 'Forecast', url: 'https://example.com/forecast', encrypted_index: 'synthetic-index' },
      { type: 'page_location', cited_text: 'Rain', document_index: 1,
        document_title: 'Rainfall', file_id: null, start_page_number: 1, end_page_number: 2 },
    ];
    const value = reply(protocol, { content: [
      { type: 'text', text: 'Snow!', citations: citations.slice(0, 2) },
      { type: 'text', text: ' Empty.', citations: [] },
      { type: 'text', text: ' Null.', citations: null },
      { type: 'text', text: ' Omitted.' },
      { type: 'text', text: ' Rain.', citations: [citations[2]] },
    ] });
    const setup = await fixture(context, protocol, viaRemote, { value });
    const input = { ...protocol.body, temperature: 0.25 };
    for (const stream of [false, true]) {
      const result = await setup.post({ ...input, stream });
      assert.equal(result.status, 200);
      if (!stream) {
        assert.equal(result.body.toString(), JSON.stringify(value));
        continue;
      }
      const received = events(result.body);
      const citationEvents = received.filter(item => item.delta?.type === 'citations_delta');
      assert.deepEqual(citationEvents.map(({ index, delta }) => ({ index, citation: delta.citation })), [
        { index: 0, citation: citations[0] },
        { index: 0, citation: citations[1] },
        { index: 4, citation: citations[2] },
      ]);
      const starts = received.filter(item => item.type === 'content_block_start');
      assert.deepEqual(starts.map(item => item.content_block), [
        { type: 'text', text: '', citations: [] },
        { type: 'text', text: '', citations: [] },
        { type: 'text', text: '', citations: null },
        { type: 'text', text: '' },
        { type: 'text', text: '', citations: [] },
      ]);
      assert.deepEqual(starts.map(item => item.index), [0, 1, 2, 3, 4]);
      assert.deepEqual(received.filter(item => item.delta?.type === 'text_delta')
        .map(item => item.delta.text), value.content.map(item => item.text));
      for (const citation of citationEvents) {
        const text = received.findIndex(item => item.index === citation.index && item.delta?.type === 'text_delta');
        const stop = received.findIndex(item => item.index === citation.index && item.type === 'content_block_stop');
        assert.ok(text < received.indexOf(citation) && received.indexOf(citation) < stop);
      }
      assert.equal(received[0].type, 'message_start');
      assert.deepEqual(received[0].message.content, []);
      assert.deepEqual(received.at(-2), { type: 'message_delta',
        delta: { stop_reason: value.stop_reason, stop_sequence: value.stop_sequence }, usage: value.usage });
      assert.equal(received.at(-1).type, 'message_stop');
    }
    assert.equal(setup.observed.length, 2);
    for (const observed of setup.observed) {
      assert.equal(observed.path, protocol.upstream);
      assert.deepEqual(observed.body.request, { ...input, stream: false });
      assert.equal(observed.headers.authorization, 'Bearer synthetic-upstream');
    }
  });
}

function events(bytes) {
  return bytes.toString().split('\n\n').flatMap(frame => {
    const data = frame.split('\n').find(line => line.startsWith('data: '));
    if (!data) return [];
    const event = JSON.parse(data.slice(6));
    assert.ok(frame.includes(`event: ${event.type}\n`));
    return [event];
  });
}

for (const viaRemote of [false, true]) {
  test(`Responses text events preserve log probabilities, remote=${viaRemote}`, async context => {
    const protocol = protocols.find(value => value.name === 'responses');
    const logprobs = [
      { token: 'Snow', logprob: -0.125, top_logprobs: [{ token: 'Rain', logprob: -2.5 }] },
      { token: '!', logprob: -0.25, top_logprobs: [{ token: '.', logprob: -1.5 }] },
    ];
    const value = reply(protocol, { usage: { input_tokens: 1, output_tokens: 2, total_tokens: 3 },
      output: [{ type: 'message', id: 'synthetic-message', role: 'assistant', status: 'completed',
        content: [{ type: 'output_text', text: 'Snow!', annotations: [], logprobs }] }] });
    const setup = await fixture(context, protocol, viaRemote, { value });
    const input = { ...protocol.body, include: ['message.output_text.logprobs'], top_logprobs: 2 };
    for (const stream of [false, true]) {
      const result = await setup.post({ ...input, stream });
      assert.equal(result.status, 200);
      if (!stream) {
        assert.equal(result.body.toString(), JSON.stringify(value));
        continue;
      }
      const received = events(result.body);
      assert.deepEqual(received.find(item => item.type === 'response.output_text.delta').logprobs, logprobs);
      assert.deepEqual(received.find(item => item.type === 'response.output_text.done').logprobs, logprobs);
      assert.deepEqual(received.find(item => item.type === 'response.content_part.added').part.logprobs, []);
      assert.deepEqual(received.at(-1).response, value);
    }
    assert.equal(setup.observed.length, 2);
    for (const observed of setup.observed) {
      assert.deepEqual(observed.body.request, { ...input, stream: false });
    }
  });
}

for (const protocol of protocols) for (const viaRemote of [false, true]) {
  const name = `${protocol.name}, remote=${viaRemote}`;
  if (protocol.name === 'messages') {
    test(`missing usage retains Messages token-count fields: ${name}`, async context => {
      const setup = await fixture(context, protocol, viaRemote, {
        value: reply(protocol, { usage: undefined }),
      });
      const result = await setup.post({ ...protocol.body, stream: true });
      assert.equal(result.status, 200);
      const received = events(result.body);
      assert.deepEqual(received[0].message.usage, { input_tokens: 0, output_tokens: 0 });
      assert.deepEqual(received.at(-2).usage, { input_tokens: 0, output_tokens: 0 });
      assert.equal(received.at(-1).type, 'message_stop');
      assert.equal(received.find(item => item.delta?.type === 'text_delta').delta.text,
        'Synthetic reply.');
      assert.equal(setup.observed.length, 1);
      assert.equal(setup.observed[0].body.request.stream, false);
    });
  }
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
    if (failure.name === 'HTTP refusal') {
      const error = protocol.name === 'responses' ? received[0] : received[0].error;
      assert.equal(error.code, 'upstream_http_error');
      assert.equal(error.message, failure.payload);
      assert.equal(received[0].upstream, failure.payload);
    } else assert.equal(result.body.includes('SYNTHETIC_SECRET'), false);
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
