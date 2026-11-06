import test from 'node:test';
import assert from 'node:assert/strict';
import { Agent } from 'node:http';
import { readCustomStream } from '../.tools/scripts/fixtures/custom-stream-reply.js';
import { clientStreamModes, customRequestCases, identify, identityOf, unicodeText, withStream }
  from './custom-request-cases.mjs';
import { fixture, post, protocols, reply } from './custom-routes-fixture.mjs';

const protocolOf = entry => protocols.find(value => value.name === entry.protocol);

// The synthetic upstream echoes the request identity so client output proves which request it answers.
function replyFor(entry, request) {
  const text = `${request.model} ${identityOf(request)} ${unicodeText}`;
  const protocol = protocolOf(entry);
  return reply(protocol, { model: request.model, ...(entry.protocol === 'responses'
    ? { usage: { input_tokens: 1, output_tokens: 2, total_tokens: 3 },
      output: [{ type: 'message', id: 'synthetic-message', role: 'assistant',
        content: [{ type: 'output_text', text, annotations: [] }] }] }
    : { content: [{ type: 'text', text }] }) });
}

function echo(entryFor) {
  return (response, count, observed) => {
    const { request } = observed.body;
    response.setHeader('content-type', 'application/json');
    response.end(JSON.stringify(replyFor(entryFor(request), request)));
  };
}

// Upstream must receive exactly the client body, forced to a completed (non-streaming) request.
function assertUpstream(entry, observed, body) {
  assert.equal(observed.path, entry.upstream);
  assert.equal(observed.headers.authorization, 'Bearer synthetic-upstream');
  assert.deepEqual(observed.body.request, { ...body, stream: false });
  assert.equal(observed.body.request.stream, false);
  assert.equal(observed.body.model, body.model);
  assert.equal(observed.body.projectId, 'synthetic-project');
  assert.equal(observed.body.castleToken, 'synthetic-castle');
}

// Validate client output with the existing readers, not by trusting the upstream request.
function assertClient(entry, result, body, mode) {
  assert.equal(result.status, 200);
  const expected = replyFor(entry, body);
  if (mode !== 'true') {
    assert.match(result.headers['content-type'], /^application\/json/);
    assert.deepEqual(JSON.parse(result.body), expected);
    return;
  }
  assert.match(result.headers['content-type'], /^text\/event-stream/);
  assert.equal(result.headers['x-should-retry'], 'false');
  assert.deepEqual(readCustomStream(entry.protocol, result.body), expected);
}

for (const entry of customRequestCases) for (const viaRemote of [false, true]) {
  for (const mode of clientStreamModes) {
    test(`request case ${entry.name}, stream=${mode}, remote=${viaRemote}`, async context => {
      const protocol = protocolOf(entry);
      const setup = await fixture(context, protocol, viaRemote, { respond: echo(() => entry) });
      const body = withStream(entry.body, mode);
      assert.equal('stream' in body, mode !== 'omitted');
      const result = await setup.post(body);
      assertClient(entry, result, entry.body, mode);
      assert.equal(setup.observed.length, 1);
      assertUpstream(entry, setup.observed[0], entry.body);
      assert.equal(setup.counts.auth, 1);
      assert.equal(setup.counts.mints, 1);
      assert.deepEqual(setup.serverCalls, [{ path: entry.path,
        authorization: viaRemote ? 'Bearer synthetic-server' : 'Bearer synthetic-client' }]);
    });
  }
}

// Hold every upstream reply until all requests arrive, then finish them in reverse arrival order.
function reversedBarrier(expected, entryFor) {
  const held = [];
  const finished = [];
  const respond = (response, count, observed) => {
    // A replay or extra request is refused; the exact observed count then fails the test.
    if (held.length === expected) { response.destroy(); return; }
    held.push({ response, observed });
    if (held.length < expected) return;
    const release = index => {
      if (index < 0) return;
      const { response: next, observed: { body: { request } } } = held[index];
      next.setHeader('content-type', 'application/json');
      next.end(JSON.stringify(replyFor(entryFor(request), request)), () => {
        finished.push(identityOf(request));
        release(index - 1);
      });
    };
    release(held.length - 1);
  };
  return { held, finished, respond };
}

for (const viaRemote of [false, true]) {
  test(`mixed-model concurrent requests stay isolated, remote=${viaRemote}`, async context => {
    const agent = new Agent({ keepAlive: false, proxyEnv: {} });
    context.after(() => agent.destroy());
    const plan = Array.from({ length: 8 }, (_, index) => {
      const entry = customRequestCases[index % customRequestCases.length];
      const id = `concurrent-${index}`;
      return { entry, id, mode: clientStreamModes[index % clientStreamModes.length],
        body: identify(entry, id) };
    });
    const byId = new Map(plan.map(item => [item.id, item]));
    const barrier = reversedBarrier(plan.length, request => byId.get(identityOf(request)).entry);
    const setup = await fixture(context, protocols[0], viaRemote, { respond: barrier.respond });
    const results = await Promise.all(plan.map(item =>
      post(setup.origin, item.entry.path, withStream(item.body, item.mode), agent)));
    for (const [index, item] of plan.entries()) assertClient(item.entry, results[index], item.body, item.mode);
    assert.equal(setup.observed.length, plan.length);
    const seen = setup.observed.map(observed => identityOf(observed.body.request));
    assert.deepEqual([...seen].sort(), plan.map(item => item.id).sort());
    for (const observed of setup.observed) {
      const item = byId.get(identityOf(observed.body.request));
      assertUpstream(item.entry, observed, item.body);
    }
    assert.deepEqual(barrier.finished, [...seen].reverse());
    assert.equal(setup.counts.auth, plan.length);
    assert.equal(setup.counts.mints, plan.length);
    assert.equal(setup.serverCalls.length, plan.length);
  });
}
