import test from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { once } from 'node:events';
import { Readable } from 'node:stream';
import { createProvider } from '../dist/provider.js';
import { ConfigurationError } from '../dist/errors.js';

for (const prefix of ['', '/gateway/', '/gateway%3F%23/']) {
  for (const suffix of ['', '?', '#', '?#']) {
    test(`upstream ${JSON.stringify(prefix + suffix)} preserves route paths`,
      { timeout: 5000 }, async context => {
        const calls = [];
        const server = http.createServer(async (request, response) => {
          const chunks = [];
          for await (const chunk of request) chunks.push(chunk);
          calls.push({ method: request.method, path: request.url,
            body: Buffer.concat(chunks).toString() });
          response.setHeader('content-type', 'application/json');
          response.end('{"ok":true}');
        });
        let provider;
        context.after(async () => {
          await provider?.close();
          server.closeAllConnections();
          await new Promise(resolve => server.close(resolve));
        });
        server.listen(0, '127.0.0.1');
        await once(server, 'listening');
        provider = createProvider({
          upstream: `http://127.0.0.1:${server.address().port}${prefix}${suffix}`,
        });
        assert.equal(await provider.ready(), true);
        const payload = '{"model":"synthetic-model","stream":false}';
        const paths = ['/v1/responses', '/v1/chat/completions'];
        for (const path of paths) {
          const result = await provider.forward({ path,
            body: Readable.from([Buffer.from(payload)]) });
          assert.equal(result.status, 200);
          const chunks = [];
          for await (const chunk of result.body) chunks.push(chunk);
          assert.equal(Buffer.concat(chunks).toString(), '{"ok":true}');
        }
        const basePath = prefix.replace(/\/+$/, '');
        assert.deepEqual(calls, [
          { method: 'GET', path: `${basePath}/v1/models`, body: '' },
          ...paths.map(path => ({ method: 'POST', path: basePath + path, body: payload })),
        ]);
      });
  }
}

test('upstream query and fragment values still fail without exposing them', () => {
  for (const suffix of ['?token=SYNTHETIC_PRIVATE', '#SYNTHETIC_PRIVATE',
    '?#SYNTHETIC_PRIVATE', '?token=SYNTHETIC_PRIVATE#']) {
    assert.throws(() => createProvider({ upstream: `http://127.0.0.1/${suffix}` }), error => {
      assert.ok(error instanceof ConfigurationError);
      assert.equal(error.message, 'Invalid upstream URL');
      assert.doesNotMatch(JSON.stringify(error) + String(error), /SYNTHETIC_PRIVATE/);
      return true;
    });
  }
});
