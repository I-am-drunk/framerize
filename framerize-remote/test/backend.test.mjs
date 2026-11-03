import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRemoteBackend } from '../dist/index.js';
import { startSyntheticRemote } from './synthetic-remote.mjs';

const timings = {
  headersTimeoutMs: 5000, idleTimeoutMs: 5000,
  readinessBodyMaxBytes: 64 * 1024, readinessWholeMs: 10000,
};

function backendFor(remote, token = 'remote-server-token') {
  return createRemoteBackend({
    endpoint: { origin: remote.origin, loopback: true },
    serverToken: token, ...timings,
  });
}

async function collect(body) {
  const chunks = [];
  for await (const chunk of body) chunks.push(chunk);
  return Buffer.concat(chunks).toString('utf8');
}

test('ready returns true for a contract-v1 ready remote', async () => {
  const remote = await startSyntheticRemote();
  const backend = backendFor(remote);
  try {
    const result = await backend.ready({ signal: new AbortController().signal });
    assert.equal(result, true);
    assert.deepEqual(remote.received.authorizations, ['Bearer remote-server-token']);
  } finally { backend.close(); await remote.close(); }
});

test('ready maps an unavailable document to its reason', async () => {
  const remote = await startSyntheticRemote({ ready: { apiVersion: 1, status: 'unavailable',
    reason: 'upstream_unavailable', capabilities: { responses: true, chatCompletions: true, streaming: true } } });
  const backend = backendFor(remote);
  try {
    const result = await backend.ready({ signal: new AbortController().signal });
    assert.deepEqual(result, { ready: false, reason: 'upstream_unavailable' });
  } finally { backend.close(); await remote.close(); }
});

test('forward streams bytes unchanged and sends the server token only', async () => {
  const remote = await startSyntheticRemote();
  const backend = backendFor(remote);
  try {
    const body = (async function* () { yield Buffer.from('{"model":"x"}'); })();
    const response = await backend.forward({ path: '/v1/responses',
      headers: { 'content-type': 'application/json' }, body, signal: new AbortController().signal });
    assert.equal(response.status, 200);
    const text = await collect(response.body);
    assert.match(text, /"delta":"OK"/);
    assert.match(text, /\[DONE\]/);
    assert.ok(remote.received.authorizations.every((value) => value === 'Bearer remote-server-token'));
  } finally { backend.close(); await remote.close(); }
});
