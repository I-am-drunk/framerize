import assert from 'node:assert/strict';
import test from 'node:test';
import { Agent } from 'node:http';
import { fixture, post, protocols, reply } from './custom-routes-fixture.mjs';
import { readCustomStream } from '../.tools/scripts/fixtures/custom-stream-reply.js';

function responseFor(protocol, overrides) {
  const value = reply(protocol, overrides);
  if (protocol.name === 'responses') value.usage.total_tokens = 3;
  return value;
}

function assertReply(result, protocol, expected, stream) {
  assert.equal(result.status, 200);
  const value = stream ? readCustomStream(protocol.name, result.body) : JSON.parse(result.body);
  assert.deepEqual(value, expected);
}

for (const cancelledProtocol of protocols) for (const viaRemote of [false, true]) {
  for (const stream of [false, true]) {
    test(`cancel one request without cancelling its peer: ${cancelledProtocol.name}, remote=${viaRemote}, stream=${stream}`,
      { timeout: 6000 }, async context => {
        const peerProtocol = protocols.find(value => value !== cancelledProtocol);
        const held = [];
        const started = [Promise.withResolvers(), Promise.withResolvers()];
        const closed = [Promise.withResolvers(), Promise.withResolvers()];
        const agent = new Agent({ keepAlive: false, proxyEnv: {} });
        context.after(() => agent.destroy());
        let setup;
        setup = await fixture(context, cancelledProtocol, viaRemote, { respond(response, count) {
          const observed = setup.observed[count - 1];
          const protocol = protocols.find(value => value.upstream === observed.path);
          const value = responseFor(protocol, { id: `synthetic-${count}`, model: observed.body.request.model });
          const payload = Buffer.from(JSON.stringify(value));
          response.setHeader('content-type', 'application/json');
          if (count > 2) { response.end(payload); return; }
          held.push({ response, payload });
          response.once('close', closed[count - 1].resolve);
          response.write(payload.subarray(0, 1));
          started[count - 1].resolve();
        } });
        const controller = new AbortController();
        const cancelledBody = { ...cancelledProtocol.body, model: 'synthetic-cancelled', stream };
        const cancelled = Promise.allSettled([post(setup.origin, cancelledProtocol.path,
          cancelledBody, agent, {}, controller.signal)]);
        await started[0].promise;
        const peerBody = { ...peerProtocol.body, model: 'synthetic-peer', stream };
        const peer = Promise.allSettled([post(setup.origin, peerProtocol.path, peerBody, agent)]);
        await started[1].promise;
        assert.equal(setup.observed.length, 2);
        assert.equal(setup.counts.mints, 2);
        controller.abort();
        await closed[0].promise;
        const [cancelledResult] = await cancelled;
        assert.equal(cancelledResult.status, 'rejected');
        assert.equal(cancelledResult.reason.name, 'AbortError');
        assert.equal(setup.signals[0].aborted, true);
        assert.equal(setup.signals[1].aborted, false);
        assert.equal(held[1].response.destroyed, false);
        held[1].response.end(held[1].payload.subarray(1));
        const [peerResult] = await peer;
        assert.equal(peerResult.status, 'fulfilled', peerResult.reason?.message);
        assertReply(peerResult.value, peerProtocol,
          responseFor(peerProtocol, { id: 'synthetic-2', model: peerBody.model }), stream);
        const recoveredBody = { ...cancelledProtocol.body, model: 'synthetic-recovered', stream };
        const recovered = await setup.post(recoveredBody);
        assertReply(recovered, cancelledProtocol,
          responseFor(cancelledProtocol, { id: 'synthetic-3', model: recoveredBody.model }), stream);
        assert.deepEqual(setup.observed.map(value =>
          [value.path, value.body.model, value.body.request.model, value.body.request.stream]), [
          [cancelledProtocol.upstream, cancelledBody.model, cancelledBody.model, false],
          [peerProtocol.upstream, peerBody.model, peerBody.model, false],
          [cancelledProtocol.upstream, recoveredBody.model, recoveredBody.model, false],
        ]);
        assert.equal(setup.counts.auth, 3);
        assert.equal(setup.counts.mints, 3);
      });
  }
}
