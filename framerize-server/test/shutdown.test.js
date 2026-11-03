import test from 'node:test';
import assert from 'node:assert/strict';
import {
  apiRequest, controlledStream, deferred, JSON_BODY,
  openSocket, rawPost, socketChunks, startFixture,
} from './helpers/fixtures.js';
import { createServer } from '../src/server.js';
import { loadConfig } from '../src/config.js';

test('graceful shutdown cancels active work, closes provider and releases the port', async (t) => {
  const { app, port, provider } = await startFixture(t);
  const aborted = deferred();
  const stream = controlledStream();
  provider.forwardImpl = ({ signal }) => {
    signal.addEventListener('abort', () => aborted.resolve(signal.reason), { once: true });
    return { status: 200, headers: { 'content-type': 'text/event-stream' }, body: stream };
  };
  const socket = await openSocket(port);
  t.after(() => socket.destroy());
  const received = socketChunks(socket);
  socket.write(rawPost() + JSON_BODY);
  stream.write('data: active\n\n');
  await received.waitFor('data: active');
  await app.close();
  await aborted.promise;
  assert.equal(provider.calls[0].signal.aborted, true);
  assert.equal(provider.closeCalls, 1);
  assert.equal(app.server.listening, false);
  await app.close();
  assert.equal(provider.closeCalls, 1);
  await assert.rejects(apiRequest(port, '/health'));
});

test('closing before listen prevents a later bind', async () => {
  const provider = { closeCalls: 0, async close() { this.closeCalls += 1; } };
  const app = await createServer(loadConfig({ port: 0, serverToken: 'fixture' }), { provider });
  await app.close();
  await assert.rejects(app.listen(), /closing/);
  assert.equal(app.server.listening, false);
  assert.equal(provider.closeCalls, 1);
});

test('hung provider shutdown respects its deadline', async () => {
  const provider = { close() { return new Promise(() => {}); } };
  const app = await createServer(loadConfig({
    port: 0, serverToken: 'fixture', shutdownTimeoutMs: 20,
  }), { provider });
  await assert.rejects(app.close(), /shutdown timed out/);
  assert.equal(app.server.listening, false);
});
