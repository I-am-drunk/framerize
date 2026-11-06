import assert from 'node:assert/strict';
import test from 'node:test';
import { fixture, protocols } from './custom-routes-fixture.mjs';

const limit = 1024;
function requestAtLimit(protocol) {
  const body = { ...protocol.body, metadata: { padding: 'Unicode é🙂' } };
  body.metadata.padding += 'x'.repeat(limit - Buffer.byteLength(JSON.stringify(body)));
  assert.equal(Buffer.byteLength(JSON.stringify(body)), limit);
  assert.ok(JSON.stringify(body).length < limit);
  return body;
}

for (const protocol of protocols) for (const viaRemote of [false, true]) {
  for (const stage of ['server', 'backend']) {
    test(`custom request byte limit at ${stage}: ${protocol.name}, remote=${viaRemote}`,
      async context => {
        const setup = await fixture(context, protocol, viaRemote,
          { [stage]: { maxBodyBytes: limit } });
        const body = requestAtLimit(protocol);
        const oversized = { ...body, metadata: { padding: body.metadata.padding + 'x' } };
        assert.equal(Buffer.byteLength(JSON.stringify(oversized)), limit + 1);
        const rejected = await setup.post(oversized);
        assert.equal(rejected.status, 413);
        assert.equal(JSON.parse(rejected.body).error.code, 'body_too_large');
        assert.equal(setup.observed.length, 0);
        assert.equal(setup.counts.auth, 0);
        assert.equal(setup.counts.mints, 0);
        const accepted = await setup.post(body);
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
