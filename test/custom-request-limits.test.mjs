import assert from 'node:assert/strict';
import test from 'node:test';
import { request } from 'node:http';
import { fixture, protocols } from './custom-routes-fixture.mjs';

const limit = 1024;
for (const protocol of protocols) for (const viaRemote of [false, true]) {
  test(`unfinished upload cannot start inference: ${protocol.name}, remote=${viaRemote}`, async context => {
    const setup = await fixture(context, protocol, viaRemote);
    let rejectUpload;
    const uploaded = new Promise((resolve, reject) => {
      rejectUpload = reject;
      setup.listener.once('request', incoming => {
        const closed = new Promise(done => incoming.once('close', done));
        incoming.once('error', reject);
        incoming.once('data', () => resolve({ closed }));
      });
    });
    const outgoing = request(new URL(protocol.path, setup.origin), { agent: false, method: 'POST',
      headers: { authorization: 'Bearer synthetic-client', 'content-type': 'application/json',
        'transfer-encoding': 'chunked' } });
    outgoing.once('error', rejectUpload);
    outgoing.setTimeout(2000, () => outgoing.destroy(new Error('fixture upload deadline')));
    const clientClosed = new Promise(resolve => outgoing.once('close', resolve));
    try {
      outgoing.write(JSON.stringify(protocol.body));
      const { closed } = await uploaded;
      assert.equal(setup.counts.auth, 0);
      assert.equal(setup.counts.mints, 0);
      outgoing.destroy();
      await Promise.all([closed, clientClosed]);
      assert.equal(setup.observed.length, 0);
      assert.equal(setup.counts.auth, 0);
      assert.equal(setup.counts.mints, 0);
      const recovered = await setup.post();
      assert.equal(recovered.status, 200);
      assert.deepEqual(recovered.body, setup.payload);
      assert.equal(setup.observed.length, 1);
      assert.equal(setup.counts.auth, 1);
      assert.equal(setup.counts.mints, 1);
    } finally { outgoing.destroy(); }
  });
}

function requestAtLimit(protocol) {
  const body = { ...protocol.body, metadata: { padding: 'Unicode é🙂' } };
  body.metadata.padding += 'x'.repeat(limit - Buffer.byteLength(JSON.stringify(body)));
  assert.equal(Buffer.byteLength(JSON.stringify(body)), limit);
  assert.ok(JSON.stringify(body).length < limit);
  return body;
}

for (const protocol of protocols) for (const viaRemote of [false, true]) {
  const stages = [['server', 'length'], ['server', 'chunked'], ['backend', 'length']];
  if (viaRemote) stages.push(['remoteListener', 'length'], ['remoteListener', 'chunked']);
  for (const [stage, framing] of stages) {
    test(`custom request byte limit at ${stage}, client framing=${framing}: ${protocol.name}, remote=${viaRemote}`,
      async context => {
        const headers = framing === 'chunked' ? { 'transfer-encoding': 'chunked' } : {};
        const setup = await fixture(context, protocol, viaRemote,
          { [stage]: { maxBodyBytes: limit } });
        const body = requestAtLimit(protocol);
        const oversized = { ...body, metadata: { padding: body.metadata.padding + 'x' } };
        assert.equal(Buffer.byteLength(JSON.stringify(oversized)), limit + 1);
        const rejected = await setup.post(oversized, headers);
        assert.equal(rejected.status, 413);
        assert.equal(JSON.parse(rejected.body).error.code, 'body_too_large');
        assert.equal(setup.observed.length, 0);
        assert.equal(setup.counts.auth, 0);
        assert.equal(setup.counts.mints, 0);
        assert.equal(setup.serverCalls.length, stage === 'remoteListener' ? 0 : 1);
        const accepted = await setup.post(body, headers);
        assert.equal(accepted.status, 200);
        assert.deepEqual(accepted.body, setup.payload);
        assert.equal(setup.observed.length, 1);
        assert.equal(setup.observed[0].path, protocol.upstream);
        assert.deepEqual(setup.observed[0].body.request, body);
        assert.equal(setup.counts.auth, 1);
        assert.equal(setup.counts.mints, 1);
      });
  }
}
