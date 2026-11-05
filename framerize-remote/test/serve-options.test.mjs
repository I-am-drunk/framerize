import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer, request, Server } from 'node:http';
import { once, getEventListeners } from 'node:events';
import { mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
const { run } = await import(process.env.REMOTE_SERVE_ENTRY ?? '../dist/index.js');

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/listenOptions.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
const { applyListenOverrides, parseListenOptions } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const quiet = { stdout() {}, stderr() {} };
const ready = { apiVersion: 1, status: 'ready',
  capabilities: { responses: true, chatCompletions: true, streaming: true } };
async function remotePeer(context, available = true, delays = {}) {
  const calls = [];
  const server = createServer((incoming, outgoing) => {
    calls.push({ path: incoming.url, authorization: incoming.headers.authorization });
    incoming.resume();
    if (!available) {
      outgoing.statusCode = 503;
      outgoing.end(JSON.stringify({ ...ready, status: 'unavailable', reason: 'upstream_unavailable' }));
      return;
    }
    const isReady = incoming.url === '/ready';
    const timer = setTimeout(() => outgoing.end(isReady ? JSON.stringify(ready) : 'opaque inference'),
      (isReady ? delays.readiness : delays.inference) ?? 0);
    outgoing.once('close', () => clearTimeout(timer));
  });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  context.after(async () => {
    server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
  });
  return { calls, port: server.address().port, origin: `http://127.0.0.1:${server.address().port}` };
}

async function credentials(root) {
  const serverToken = { kind: 'file', path: join(root, 'server.token') };
  const localToken = { kind: 'file', path: join(root, 'local.token') };
  await writeFile(serverToken.path, 'synthetic-server'); await writeFile(localToken.path, 'synthetic-local');
  return { serverToken, localToken };
}

function exchange(port, token) {
  return new Promise((resolve, reject) => {
    const outgoing = request({ host: '127.0.0.1', port, path: '/v1/responses', method: 'POST',
      agent: false, headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' } }, response => {
      const chunks = [];
      response.on('data', chunk => chunks.push(chunk)); response.on('error', reject);
      response.on('end', () => resolve({ status: response.statusCode, body: Buffer.concat(chunks).toString() }));
    });
    outgoing.on('error', reject); outgoing.end('{"model":"fixture"}');
  });
}

for (const [label, delays, expected] of [
  ['reserves time for readiness', { readiness: 600, inference: 40 }, 200],
  ['returns the inference deadline without retrying', { inference: 800 }, 504],
]) {
  test(`serve ${label}`, { timeout: 5000 }, async context => {
    const remote = await remotePeer(context, true, delays);
    const { root, path } = await profile(context, config);
    await writeFile(path, JSON.stringify({ ...config, remote: remote.origin,
      inferenceTimeoutMs: 250, ...await credentials(root) }));
    let started;
    const listening = new Promise(resolve => { started = resolve; });
    const serving = run(['serve', '--config', path, '--port', '0', '--json'], {
      ...quiet, stdout(text) { const event = JSON.parse(text); if (event.listening) started(event.listening.port); },
    });
    try {
      const port = await Promise.race([listening, serving.then(() => { throw new Error('Serve exited'); })]);
      const response = await fetch(`http://127.0.0.1:${port}/v1/responses`, {
        method: 'POST', signal: AbortSignal.timeout(3000),
        headers: { authorization: 'Bearer synthetic-local', 'content-type': 'application/json' },
        body: '{"model":"fixture","stream":false}',
      });
      const body = await response.text();
      assert.equal(response.status, expected, body);
      if (expected === 200) assert.equal(body, 'opaque inference');
      else {
        assert.equal(response.headers.get('x-should-retry'), 'false');
        assert.match(body, /headers_timeout/);
      }
      assert.deepEqual(remote.calls.map(call => call.path), ['/ready', '/ready', '/v1/responses']);
    } finally { process.emit('SIGTERM'); assert.equal(await serving, 0); }
  });
}

test('serve help needs no profile or credentials', async () => {
  let output = '';
  assert.equal(await run(['serve', '--config', '/synthetic/missing-profile', '--help'], {
    ...quiet, stdout(text) { output += text; },
  }), 0);
  assert.match(output, /--host ADDRESS/); assert.match(output, /Overrides never change the saved profile/);
});

test('a bind collision has a fixed error and cleans up signal handlers', async context => {
  const remote = await remotePeer(context);
  const { root, path } = await profile(context, config);
  await writeFile(path, JSON.stringify({ ...config, remote: remote.origin, port: remote.port,
    ...await credentials(root) }));
  const listeners = ['SIGINT', 'SIGTERM'].map(name => getEventListeners(process, name).length);
  await assert.rejects(run(['serve', '--config', path], quiet), { code: 'listen_failed' });
  assert.equal(remote.calls.length, 1);
  assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => getEventListeners(process, name).length), listeners);
  assert.equal((await exchange(remote.port, 'synthetic-server')).status, 200);
});

test('exposure consent never bypasses remote readiness or opens an early listener', async context => {
  const remote = await remotePeer(context, false);
  const { root, path } = await profile(context, config);
  await writeFile(path, JSON.stringify({ ...config, remote: remote.origin, ...await credentials(root) }));
  const before = await readFile(path);
  context.mock.method(Server.prototype, 'listen', () => assert.fail('must not listen'));
  let output = '';
  assert.equal(await run(['serve', '--config', path, '--host', '0.0.0.0', '--allow-non-loopback', '--json'], {
    ...quiet, stdout(text) { output += text; },
  }), 1);
  const result = JSON.parse(output);
  assert.equal(result.ready, false); assert.equal(result.reason, 'upstream_unavailable');
  assert.equal(result.exposure, undefined); assert.equal(result.listening, undefined);
  assert.equal(remote.calls.length, 1); assert.deepEqual(await readFile(path), before);
});

for (const json of [false, true]) {
  test(`temporary exposure keeps auth and reports ${json ? 'JSON' : 'compact'} output`, { timeout: 5000 }, async context => {
    const remote = await remotePeer(context);
    const { root, path } = await profile(context, config);
    await writeFile(path, JSON.stringify({ ...config, remote: remote.origin, port: remote.port,
      ...await credentials(root) }));
    const before = await readFile(path);
    const original = Server.prototype.listen;
    let observedHost;
    context.mock.method(Server.prototype, 'listen', function(port, host, ...rest) {
      observedHost = host;
      assert.equal(host, '0.0.0.0');
      return original.call(this, port, '127.0.0.1', ...rest);
    });
    const listeners = ['SIGINT', 'SIGTERM'].map(name => getEventListeners(process, name).length);
    let output = ''; let started;
    const listening = new Promise(resolve => { started = resolve; });
    const serving = run(['serve', '--config', path, '--host', '0.0.0.0', '--port', '0',
      '--allow-non-loopback', ...(json ? ['--json'] : [])], { ...quiet, stdout(text) {
      output += text;
      const event = json ? JSON.parse(text) : undefined;
      const match = text.match(/Local API: http:\/\/127\.0\.0\.1:(\d+)\/v1/);
      if (event?.listening || match) started(event?.listening.port ?? Number(match[1]));
    } });
    let port;
    try {
      port = await Promise.race([listening, serving.then(() => { throw new Error('Serve ended before listening'); })]);
      assert.notEqual(port, remote.port); assert.equal(observedHost, '0.0.0.0');
      assert.equal((await exchange(port, 'synthetic-wrong')).status, 401);
      assert.equal(remote.calls.length, 1);
      assert.deepEqual(await exchange(port, 'synthetic-local'), { status: 200, body: 'opaque inference' });
      assert.deepEqual(remote.calls.map(call => call.path), ['/ready', '/ready', '/v1/responses']);
      assert.ok(remote.calls.every(call => call.authorization === 'Bearer synthetic-server'));
    } finally { process.emit('SIGTERM'); assert.equal(await serving, 0); }
    await assert.rejects(exchange(port, 'synthetic-local'), { code: 'ECONNREFUSED' });
    assert.deepEqual(await readFile(path), before);
    assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => getEventListeners(process, name).length), listeners);
    assert.doesNotMatch(output, /synthetic-(?:local|server)/);
    if (json) assert.deepEqual(output.trim().split('\n').map(JSON.parse), [
      { ready: true, listening: { host: '0.0.0.0', port }, exposure: 'non-loopback-http' }, { stopped: true },
    ]);
    else assert.deepEqual(output.trim().split('\n'), ['Remote readiness passed.',
      `Non-loopback HTTP enabled on port ${port}.`, `Local API: http://127.0.0.1:${port}/v1`, 'Stop: Ctrl+C', 'Stopped.']);
  });
}
async function profile(context, value) {
  const root = await mkdtemp(join(tmpdir(), 'remote-serve-options-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  const path = join(root, 'profile.json');
  await writeFile(path, JSON.stringify(value));
  return { root, path };
}

for (const args of [['--host', 'bad/path'],
  ['--port', '-1'], ['--port', '65536'], ['--port', '1e3'], ['--port', '0x50'], ['--port', ' 80'],
  ['--port', '80', '--port', '81'], ['--remote', 'http://127.0.0.1'], ['--server-token-env', 'SYNTHETIC']]) {
  test(`serve rejects ${args.join(' ')} before credentials or profile changes`, async context => {
    const { path } = await profile(context, config);
    const before = await readFile(path);
    await assert.rejects(run(['serve', '--config', path, ...args], quiet), error => {
      if (args[0] === '--host') assert.equal(error.code, 'listen_invalid');
      else if (args[0] === '--port' && args.length === 2) assert.equal(error.code, 'listen_invalid');
      else assert.match(error.message, /^(unknown|duplicate) option$/);
      return true;
    });
    assert.deepEqual(await readFile(path), before);
  });
}
const config = Object.freeze({ remote: 'http://127.0.0.1:1',
  serverToken: Object.freeze({ kind: 'env', name: 'MISSING_SERVER' }),
  localToken: Object.freeze({ kind: 'env', name: 'MISSING_LOCAL' }),
  host: '127.0.0.1', port: 8788, allowNonLoopback: false, allowInsecureRemote: false });

test('serve overrides are temporary, normalized and keep all remote fields', () => {
  const before = JSON.stringify(config);
  for (const [host, normalized] of [['LOCALHOST.', 'localhost'], ['[::1]', '::1'], ['127.0.0.2', '127.0.0.2']]) {
    assert.deepEqual(applyListenOverrides(config, { host, port: 0 }), { ...config, host: normalized, port: 0 });
  }
  assert.equal(JSON.stringify(config), before);
  assert.deepEqual(parseListenOptions({ port: '0', host: '::', 'allow-non-loopback': true }),
    { port: 0, host: '::', allowNonLoopback: true });
});

test('non-loopback hosts need no separate exposure consent', () => {
  for (const host of ['0.0.0.0', '::', '100.64.0.10', 'server.example']) {
    assert.equal(applyListenOverrides(config, { host }).host, host);
  }
});
