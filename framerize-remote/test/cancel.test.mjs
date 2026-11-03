import { test } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { createInstance } from '@framerize/core';
import { createRemoteBackend } from '../dist/index.js';
import { startSyntheticRemote } from './synthetic-remote.mjs';

const timings = {
  headersTimeoutMs: 5000, idleTimeoutMs: 5000,
  readinessBodyMaxBytes: 65536, readinessWholeMs: 10000,
};

function slowStream(response) {
  response.write('data: {"type":"response.output_text.delta","delta":"OK"}\n\n');
}

test('client disconnect mid-stream cancels the remote request', async () => {
  const remote = await startSyntheticRemote({ stream: slowStream });
  const backend = createRemoteBackend({
    endpoint: { origin: remote.origin, loopback: true }, serverToken: 'remote-server-token', ...timings,
  });
  const instance = createInstance({ backend, token: 'local-client-key', host: '127.0.0.1', port: 0 });
  const { port } = await instance.listen();
  try {
    await new Promise((resolve, reject) => {
      const req = http.request({ host: '127.0.0.1', port, path: '/v1/responses', method: 'POST',
        headers: { authorization: 'Bearer local-client-key', 'content-type': 'application/json' } }, (res) => {
        res.once('data', () => { req.destroy(); resolve(); });
        res.on('error', () => {});
      });
      req.on('error', () => {});
      req.end('{"model":"x"}');
    });
    await new Promise((resolve) => setTimeout(resolve, 200));
    assert.ok(remote.received.cancelled >= 1, 'remote should observe an early close');
  } finally { await instance.close(); await remote.close(); }
});
