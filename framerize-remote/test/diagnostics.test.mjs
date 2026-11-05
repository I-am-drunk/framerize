import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { createServer as createTcpServer } from 'node:net';
import { once, getEventListeners } from 'node:events';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { build } from 'esbuild';
import { createRemoteBackend, run } from '../dist/index.js';

const ready = { apiVersion: 1, status: 'ready',
  capabilities: { responses: true, chatCompletions: true, streaming: true } };
const cases = [
  [200, ready, undefined], [401, 'private body', 'remote_auth'], [403, ready, 'remote_auth'],
  [302, ready, 'http_refused'], [500, ready, 'http_refused'],
  [200, '{invalid', 'malformed_response'], [200, { ...ready, apiVersion: 2 }, 'incompatible_response'],
  [200, 'x'.repeat(65537), 'response_too_large'],
  ...['upstream_unavailable', 'configuration_unavailable', 'credentials_unavailable'].map(reason =>
    [503, { ...ready, status: 'unavailable', reason }, reason]),
];

for (const [status, body, code] of cases) {
  test(`readiness ${status} reports ${code ?? 'ready'} without remote details`, async context => {
    let calls = 0;
    const remote = await peer(context, (request, response) => {
      calls += 1; request.resume(); response.statusCode = status;
      response.setHeader('location', 'http://127.0.0.1/never-follow');
      response.setHeader('x-request-id', canary);
      response.end(typeof body === 'string' ? body : JSON.stringify(body));
    });
    const instance = backend(context, remote.origin);
    const controller = new AbortController();
    const result = await instance.diagnose({ signal: controller.signal });
    assert.equal(result.code, code);
    assert.equal(calls, 1);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    assert.doesNotMatch(JSON.stringify(result), /synthetic-private|private body|127\.0\.0\.1|never-follow/);
    assert.deepEqual(await instance.ready({ signal: controller.signal }), result.readiness);
    if (!code) assert.equal(result.readiness, true);
    else assert.equal(result.readiness.ready, false);
  });
}
const canary = 'synthetic-private-canary';
test('response header cap has a fixed diagnostic even with raised process defaults', async context => {
  const remote = await peer(context, (request, response) => {
    request.resume(); response.setHeader('x-large', 'x'.repeat(17000));
    response.end(JSON.stringify(ready));
  });
  const controller = new AbortController();
  assert.equal((await backend(context, remote.origin).diagnose({ signal: controller.signal })).code, 'response_too_large');
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});
test('check prints actionable human and JSON diagnostics using one exchange each', async context => {
  const root = await mkdtemp(join(tmpdir(), 'remote-check-diagnostic-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  let calls = 0;
  const remote = await peer(context, (request, response) => {
    calls += 1; request.resume(); response.statusCode = 401; response.end(canary);
  });
  const token = join(root, 'token'); await writeFile(token, canary);
  const profile = join(root, 'profile.json');
  await writeFile(profile, JSON.stringify({ remote: remote.origin,
    serverToken: { kind: 'file', path: token }, localToken: { kind: 'file', path: token } }));
  const listeners = ['SIGINT', 'SIGTERM'].map(name => getEventListeners(process, name).length);
  for (const json of [false, true]) {
    let output = '';
    assert.equal(await run(['check', '--config', profile, ...(json ? ['--json'] : [])], {
      stdout(text) { output += text; }, stderr() { assert.fail('unexpected stderr'); },
    }), 1);
    assert.doesNotMatch(output, /synthetic-private|127\.0\.0\.1/);
    if (json) assert.equal(JSON.parse(output).diagnostic.code, 'remote_auth');
    else assert.match(output, /^Not ready:.*\nNext: /);
  }
  assert.equal(calls, 2);
  assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => getEventListeners(process, name).length), listeners);
});
for (const kind of ['profile_missing', 'profile_invalid', 'server_credential', 'local_credential']) {
  test(`launcher identifies ${kind} without printing credential references`, async context => {
    const root = await mkdtemp(join(tmpdir(), 'remote-diagnostic-'));
    context.after(() => rm(root, { recursive: true, force: true }));
    const profile = join(root, `${canary}.json`);
    const config = { remote: 'http://127.0.0.1:1',
      serverToken: { kind: 'env', name: 'TEST_SERVER' }, localToken: { kind: 'env', name: 'TEST_LOCAL' } };
    if (kind !== 'profile_missing') await writeFile(profile, kind === 'profile_invalid' ? canary : JSON.stringify(config));
    const launcher = fileURLToPath(new URL('../bin/framerize-remote.js', import.meta.url));
    const result = spawnSync(process.execPath, [launcher, 'check', '--config', profile, '--json'], {
      encoding: 'utf8', cwd: root, timeout: 5000,
      env: { HOME: root, USERPROFILE: root, TMPDIR: root, NODE_DISABLE_COMPILE_CACHE: '1',
        ...(kind === 'local_credential' ? { TEST_SERVER: canary } : {}) },
    });
    assert.equal(result.status, 1); assert.equal(result.stdout, '');
    assert.equal(JSON.parse(result.stderr).error, kind);
    assert.doesNotMatch(result.stderr, /synthetic-private|TEST_SERVER|TEST_LOCAL|127\.0\.0\.1/);
  });
}
test('actual TLS negotiation failure is distinct from refused TCP', async context => {
  const remote = await peer(context, socket => {
    socket.once('data', () => socket.end('HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n'));
  }, createTcpServer);
  const secured = backend(context, remote.origin.replace('http:', 'https:'));
  assert.equal((await secured.diagnose({ signal: new AbortController().signal })).code, 'tls_failed');
  const reserved = createServer();
  reserved.listen(0, '127.0.0.1'); await once(reserved, 'listening');
  const origin = `http://127.0.0.1:${reserved.address().port}`;
  await new Promise(resolve => reserved.close(resolve));
  const refused = backend(context, origin);
  assert.equal((await refused.diagnose({ signal: new AbortController().signal })).code, 'connection_failed');
});
async function peer(context, handler, factory = createServer) {
  const server = factory(handler);
  const sockets = new Set();
  server.on('connection', socket => { sockets.add(socket); socket.once('close', () => sockets.delete(socket)); });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  context.after(async () => {
    for (const socket of sockets) socket.destroy();
    await new Promise(resolve => server.close(resolve));
  });
  return { origin: `http://127.0.0.1:${server.address().port}` };
}

function backend(context, origin, limits = {}) {
  const instance = createRemoteBackend({ endpoint: { origin, loopback: true }, serverToken: canary,
    headersTimeoutMs: 1000, idleTimeoutMs: 1000, readinessWholeMs: 2000,
    readinessBodyMaxBytes: 65536, ...limits });
  context.after(() => instance.close());
  return instance;
}

for (const kind of ['headers_timeout', 'idle_timeout', 'readiness_timeout', 'aborted']) {
  test(`diagnostic preserves ${kind} and removes the abort listener`, { timeout: 5000 }, async context => {
    const controller = new AbortController();
    const remote = await peer(context, (request, response) => {
      request.resume();
      if (kind !== 'headers_timeout') response.write('{');
      if (kind === 'aborted') controller.abort();
    });
    const limits = kind === 'headers_timeout' ? { headersTimeoutMs: 40 }
      : kind === 'idle_timeout' ? { idleTimeoutMs: 40 }
      : kind === 'readiness_timeout' ? { readinessWholeMs: 40 } : {};
    const result = await backend(context, remote.origin, limits).diagnose({ signal: controller.signal });
    assert.equal(result.code, kind);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  });
}
const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/diagnostics.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
const diagnostics = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

test('setup rollback diagnostics retain the original failure in text and JSON', () => {
  const initial = new diagnostics.RemoteDiagnosticError('aborted');
  const restore = new diagnostics.RemoteDiagnosticError('service_failed');
  const error = new diagnostics.SetupRollbackError(initial, [restore]);
  assert.deepEqual(error.cause.errors, [initial, restore]);
  const json = JSON.parse(diagnostics.commandFailure(error, true));
  assert.equal(json.error, 'service_rollback_failed');
  assert.equal(json.setup.code, 'aborted');
  assert.match(json.message, /could not be fully restored/);
  const text = diagnostics.commandFailure(error, false);
  assert.match(text, /Original failure: The command was cancelled/);
  assert.match(text, /could not be fully restored/);
});

test('network labels discard raw errors, certificates and throwing properties', () => {
  for (const [raw, code] of [['ENOTFOUND', 'dns_failed'], ['EAI_AGAIN', 'dns_failed'],
    ['ECONNREFUSED', 'connection_failed'], ['DEPTH_ZERO_SELF_SIGNED_CERT', 'tls_failed'],
    ['CERT_HAS_EXPIRED', 'tls_failed'], ['ERR_TLS_CERT_ALTNAME_INVALID', 'tls_failed'],
    ['ERR_SSL_WRONG_VERSION_NUMBER', 'tls_failed'], ['HPE_HEADER_OVERFLOW', 'response_too_large']]) {
    assert.equal(diagnostics.networkDiagnostic({ code: raw, message: canary, cert: canary }), code);
  }
  assert.equal(diagnostics.networkDiagnostic(new Proxy({}, { get() { throw new Error(canary); } })), 'connection_failed');
  assert.equal(diagnostics.networkDiagnostic({ code: canary }), 'connection_failed');
  for (const json of [true, false]) {
    const error = new diagnostics.RemoteDiagnosticError('tls_failed', canary);
    assert.doesNotMatch(diagnostics.commandFailure(error, json), /synthetic-private-canary/);
  }
});
