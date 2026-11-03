import assert from 'node:assert/strict';
import { getEventListeners, once } from 'node:events';
import { createServer } from 'node:http';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

const { createRemoteBackend, run } = await import(process.env.REMOTE_REVIEW_ENTRY ?? '../dist/index.js');
const ready = { apiVersion: 1, status: 'ready',
  capabilities: { responses: true, chatCompletions: true, streaming: true } };
const io = { stdout() {}, stderr() {} };

async function peer(handler) {
  const paths = [];
  const server = createServer((request, response) => {
    paths.push(request.url);
    request.resume();
    handler(request, response);
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return { paths, origin: `http://127.0.0.1:${server.address().port}`,
    async close() {
      server.closeAllConnections();
      await new Promise((resolve) => server.close(resolve));
    } };
}

function backendFor(remote, timings = {}) {
  return createRemoteBackend({
    endpoint: { origin: remote.origin, loopback: true }, serverToken: 'synthetic-remote-token',
    headersTimeoutMs: 2000, idleTimeoutMs: 1000, readinessWholeMs: 2000,
    readinessBodyMaxBytes: 65536, ...timings,
  });
}

function request(signal = new AbortController().signal) {
  return { path: '/v1/responses', headers: { 'content-type': 'application/json' }, signal,
    body: (async function* () { yield Buffer.from('{"model":"fixture"}'); })() };
}

async function collect(body) {
  const chunks = [];
  for await (const chunk of body) chunks.push(chunk);
  return Buffer.concat(chunks);
}

for (const kind of ['unavailable', 'unauthorized', 'oversized']) {
  test(`forward refuses ${kind} readiness without an inference attempt`, async () => {
    const remote = await peer((incoming, outgoing) => {
      if (incoming.url !== '/ready') return outgoing.end('inference must not run');
      outgoing.statusCode = kind === 'unauthorized' ? 401 : kind === 'unavailable' ? 503 : 200;
      outgoing.end(kind === 'oversized' ? Buffer.alloc(65537, 32)
        : JSON.stringify(kind === 'unavailable' ? { ...ready, status: 'unavailable',
          reason: 'upstream_unavailable' } : ready));
    });
    const backend = backendFor(remote);
    try {
      const result = await backend.forward(request());
      await collect(result.body);
      assert.equal(result.status, 503);
      assert.deepEqual(remote.paths, ['/ready']);
    } finally { backend.close(); await remote.close(); }
  });
}

test('readiness is fresh for every inference and releases the parent listener', async () => {
  const remote = await peer((incoming, outgoing) =>
    outgoing.end(incoming.url === '/ready' ? JSON.stringify(ready) : 'opaque bytes'));
  const backend = backendFor(remote);
  const controller = new AbortController();
  try {
    for (let index = 0; index < 2; index += 1) {
      const result = await backend.forward(request(controller.signal));
      assert.equal((await collect(result.body)).toString(), 'opaque bytes');
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    }
    assert.deepEqual(remote.paths, ['/ready', '/v1/responses', '/ready', '/v1/responses']);
  } finally { backend.close(); await remote.close(); }
});

test('ready alone releases the parent listener', async () => {
  const remote = await peer((incoming, outgoing) => outgoing.end(JSON.stringify(ready)));
  const backend = backendFor(remote);
  const controller = new AbortController();
  try {
    assert.equal(await backend.ready({ signal: controller.signal }), true);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  } finally { backend.close(); await remote.close(); }
});

test('whole readiness deadline includes delayed headers and a trickled body', async () => {
  const remote = await peer((incoming, outgoing) => {
    let interval;
    const headers = setTimeout(() => {
      outgoing.write('{');
      interval = setInterval(() => outgoing.write(' '), 30);
    }, 300);
    outgoing.once('close', () => { clearTimeout(headers); clearInterval(interval); });
  });
  const backend = backendFor(remote, { readinessWholeMs: 500 });
  const controller = new AbortController();
  const started = performance.now();
  try {
    const result = await backend.forward(request(controller.signal));
    assert.equal(result.status, 503);
    await collect(result.body);
    assert.deepEqual(remote.paths, ['/ready']);
    assert.ok(performance.now() - started < 700, 'deadline starts before headers');
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  } finally { backend.close(); await remote.close(); }
});

test('setup rejects invalid ports before writing a profile', async () => {
  const root = await mkdtemp(join(tmpdir(), 'remote-invalid-setup-'));
  const path = join(root, 'profile.json');
  try {
    await assert.rejects(run(['setup', '--config', path, '--remote', 'http://127.0.0.1',
      '--server-token-env', 'REMOTE_TOKEN', '--local-token-env', 'LOCAL_TOKEN',
      '--port', 'not-a-port'], io));
    await assert.rejects(readFile(path), { code: 'ENOENT' });
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('check rejects extra arguments before loading configuration', async () => {
  await assert.rejects(run(['check', '--config', '/synthetic/absent-profile',
    '--remote-token=synthetic-secret'], io), /unknown option|unexpected argument/);
});

test('setup help is available without writing configuration', async () => {
  let output = '';
  assert.equal(await run(['setup', '--help'], { ...io, stdout(text) { output += text; } }), 0);
  assert.match(output, /--server-token-env/);
});

for (const flag of ['allowNonLoopback', 'allowInsecureRemote']) {
  test(`configuration rejects non-boolean ${flag} before credential access`, async () => {
    const root = await mkdtemp(join(tmpdir(), 'remote-invalid-policy-'));
    const path = join(root, 'profile.json');
    try {
      await writeFile(path, JSON.stringify({ remote: 'http://127.0.0.1',
        serverToken: { kind: 'file', path: join(root, 'absent') },
        localToken: { kind: 'file', path: join(root, 'absent') }, [flag]: 'false' }));
      await assert.rejects(run(['check', '--config', path], io), /boolean/);
    } finally { await rm(root, { recursive: true, force: true }); }
  });
}

test('credential files reject BOM, invalid UTF-8, extra lines and oversize bytes', async () => {
  const root = await mkdtemp(join(tmpdir(), 'remote-credentials-'));
  const remote = await peer((incoming, outgoing) => outgoing.end(JSON.stringify(ready)));
  try {
    const token = join(root, 'token');
    const config = join(root, 'profile.json');
    await writeFile(config, JSON.stringify({ remote: remote.origin,
      serverToken: { kind: 'file', path: token }, localToken: { kind: 'file', path: token } }));
    for (const value of ['\uFEFFsynthetic-token', 'synthetic-token\nextra', 'a'.repeat(8193), Buffer.from([0xff])]) {
      await writeFile(token, value);
      await assert.rejects(run(['check', '--config', config], io), /credential is unavailable or invalid/);
    }
    assert.deepEqual(remote.paths, []);
    await writeFile(token, 'synthetic-token\r\n');
    assert.equal(await run(['check', '--config', config], io), 0);
  } finally { await remote.close(); await rm(root, { recursive: true, force: true }); }
});

test('environment credentials reject surrounding whitespace without making a request', async () => {
  const root = await mkdtemp(join(tmpdir(), 'remote-environment-'));
  const remote = await peer((incoming, outgoing) => outgoing.end(JSON.stringify(ready)));
  const name = `REMOTE_TEST_TOKEN_${process.pid}`;
  const previous = process.env[name];
  try {
    process.env[name] = ' synthetic-token ';
    const config = join(root, 'profile.json');
    await writeFile(config, JSON.stringify({ remote: remote.origin,
      serverToken: { kind: 'env', name }, localToken: { kind: 'env', name } }));
    await assert.rejects(run(['check', '--config', config], io), /credential is unavailable or invalid/);
    assert.deepEqual(remote.paths, []);
  } finally {
    if (previous === undefined) delete process.env[name]; else process.env[name] = previous;
    await remote.close();
    await rm(root, { recursive: true, force: true });
  }
});

test('aborting unfinished readiness releases its socket without starting inference', async () => {
  let started;
  let closed;
  const entered = new Promise((resolve) => { started = resolve; });
  const ended = new Promise((resolve) => { closed = resolve; });
  const remote = await peer((incoming, outgoing) => {
    outgoing.once('close', closed);
    outgoing.write('{');
    started();
  });
  const backend = backendFor(remote);
  const controller = new AbortController();
  try {
    const result = backend.forward(request(controller.signal));
    const rejection = assert.rejects(result, /remote request failed/);
    await entered;
    controller.abort();
    await Promise.all([rejection, ended]);
    assert.deepEqual(remote.paths, ['/ready']);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  } finally { backend.close(); await remote.close(); }
});

test('complete response headers retain a late Connection nomination below the byte cap', async () => {
  const remote = await peer((incoming, outgoing) => {
    if (incoming.url === '/ready') return outgoing.end(JSON.stringify(ready));
    outgoing.writeHead(200, ['x-request-id', 'synthetic-canary',
      ...Array.from({ length: 2100 }, () => ['x', 'a']).flat(), 'connection', 'x-request-id, close']);
    outgoing.end('opaque');
  });
  const backend = backendFor(remote);
  try {
    const response = await backend.forward(request());
    assert.match(response.headers.connection, /x-request-id/);
    assert.equal((await collect(response.body)).toString(), 'opaque');
  } finally { backend.close(); await remote.close(); }
});

test('inference streams have an idle limit but no readiness total deadline', async () => {
  const remote = await peer((incoming, outgoing) => {
    if (incoming.url === '/ready') return outgoing.end(JSON.stringify(ready));
    let count = 0;
    outgoing.write('start');
    const timer = setInterval(() => {
      outgoing.write('chunk');
      if (++count === 8) { clearInterval(timer); outgoing.end('end'); }
    }, 30);
    outgoing.once('close', () => clearInterval(timer));
  });
  const backend = backendFor(remote, { readinessWholeMs: 100, idleTimeoutMs: 150 });
  try {
    const response = await backend.forward(request());
    assert.equal((await collect(response.body)).toString(), 'start' + 'chunk'.repeat(8) + 'end');
  } finally { backend.close(); await remote.close(); }
});

test('upload backpressure stops pulling a body when the remote stops reading', async () => {
  let entered;
  const started = new Promise((resolve) => { entered = resolve; });
  const remote = await peer((incoming, outgoing) => {
    if (incoming.url === '/ready') return outgoing.end(JSON.stringify(ready));
    incoming.pause();
    entered();
  });
  const backend = backendFor(remote);
  const controller = new AbortController();
  let pulled = 0;
  const body = (async function* () {
    while (pulled < 512) { pulled += 1; yield Buffer.alloc(65536); }
  })();
  try {
    const result = backend.forward({ ...request(controller.signal), body });
    const rejection = assert.rejects(result, /remote request failed/);
    await started;
    await new Promise((resolve) => setTimeout(resolve, 50));
    assert.ok(pulled < 512, `upload consumed all ${pulled} chunks`);
    controller.abort();
    await rejection;
  } finally { controller.abort(); backend.close(); await remote.close(); }
});

test('inference header overflow rejects safely and releases the caller signal', async () => {
  const remote = await peer((incoming, outgoing) => {
    if (incoming.url === '/ready') return outgoing.end(JSON.stringify(ready));
    outgoing.setHeader('x-overflow', 'a'.repeat(17000));
    outgoing.end('synthetic-private-body');
  });
  const backend = backendFor(remote);
  const controller = new AbortController();
  try {
    await assert.rejects(backend.forward(request(controller.signal)), /^Error: remote request failed$/);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  } finally { backend.close(); await remote.close(); }
});

test('setup rejects malformed listen hosts even with exposure consent', async () => {
  const root = await mkdtemp(join(tmpdir(), 'remote-listen-host-'));
  const config = join(root, 'profile.json');
  const args = ['setup', '--config', config, '--remote', 'http://127.0.0.1',
    '--server-token-env', 'REMOTE', '--local-token-env', 'LOCAL', '--allow-non-loopback'];
  try {
    for (const host of ['example/path', 'https://example', 'example:123', 'example..com']) {
      await assert.rejects(run([...args, '--host', host], io), /configuration host is invalid/);
      await assert.rejects(readFile(config), { code: 'ENOENT' });
    }
    assert.equal(await run([...args, '--host', '[::1]'], io), 0);
  } finally { await rm(root, { recursive: true, force: true }); }
});
