import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer, get } from 'node:http';
import dns from 'node:dns';
import { connect } from 'node:net';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { serve } from './cli.ts'; export { isLoopbackHost } from './hostPolicy.ts';",
  resolveDir: fileURLToPath(new URL('../src/', import.meta.url)),
}, alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) },
bundle: true, platform: 'node', format: 'esm', write: false });
const { serve, isLoopbackHost } = await import(`data:text/javascript;base64,${
  Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
console.log(JSON.stringify({ runtime: process.version, bundle: createHash('sha256')
  .update(compiled.outputFiles[0].contents).digest('hex') }));

function bounded(promise, milliseconds = 1500) {
  let timer;
  return Promise.race([promise, new Promise((resolve, reject) => {
    timer = setTimeout(() => reject(new Error('fixture deadline exceeded')), milliseconds);
  })]).finally(() => clearTimeout(timer));
}

function health(host, port) {
  return new Promise((resolve, reject) => {
    const request = get({ host, port, path: '/health', agent: false }, response => {
      response.resume();
      response.once('end', () => resolve(response.statusCode));
      response.once('error', reject);
    });
    request.once('error', reject);
    request.setTimeout(1000, () => request.destroy(new Error('health deadline exceeded')));
  });
}

async function refused(host, port) {
  await assert.rejects(new Promise((resolve, reject) => {
    const socket = connect({ host, port });
    socket.once('error', reject);
    socket.once('connect', () => { socket.destroy(); resolve(); });
    socket.setTimeout(1000, () => socket.destroy(new Error('connection deadline exceeded')));
  }), { code: 'ECONNREFUSED' });
}

test('equivalent IPv6 loopback addresses are local', () => {
  for (const host of ['::1', '0::1', '::0001', '0:0:0:0:0:0:0:1', '[0000::0001]',
    '::ffff:127.0.0.1', '::ffff:127.255.255.255', '[::ffff:7f00:1]', 'localhost', '127.20.30.40']) {
    assert.equal(isLoopbackHost(host), true, host);
  }
});

test('wildcards, remote addresses and malformed hosts stay non-loopback', () => {
  for (const host of ['::', '0.0.0.0', '2001:db8::1', 'fc00::1', '192.0.2.1',
    '::ffff:126.0.0.1', '::ffff:128.0.0.1', '::ffff:10.0.0.1', '::gggg:1', 'localhost.example']) {
    assert.equal(isLoopbackHost(host), false, host);
  }
});

for (const host of ['0:0:0:0:0:0:0:1', '::ffff:127.0.0.1', 'fixture.invalid', '0177.0.0.1']) {
  test(`serve reports ${host} without a false exposure warning`, { timeout: 5000 }, async context => {
    const peerHost = host === '0:0:0:0:0:0:0:1' ? '::1' : '127.0.0.1';
    let lookups = 0;
    const syntheticLookup = host === 'fixture.invalid' || host === '0177.0.0.1';
    if (syntheticLookup) context.mock.method(dns, 'lookup', (name, options, callback) => {
      assert.equal(name, host);
      lookups++;
      queueMicrotask(() => options.all
        ? callback(null, [{ address: peerHost, family: 4 }])
        : callback(null, peerHost, 4));
    });
    const root = await mkdtemp(join(tmpdir(), 'frm-loopback-report-'));
    const keys = ['HOME', 'FRM_REMOTE', 'FRM_HOST', 'FRM_PORT', 'FRM_SERVER_TOKEN', 'FRM_LOCAL_TOKEN'];
    const original = Object.fromEntries(keys.map(key => [key, process.env[key]]));
    const beforeSignals = ['SIGINT', 'SIGTERM'].map(name => process.listenerCount(name));
    const sockets = new Set();
    let service, upstreamPort, localPort, finished = false;
    const upstream = createServer((request, response) => {
      response.setHeader('content-type', 'application/json');
      response.end(JSON.stringify({ apiVersion: 1, status: 'ready',
        capabilities: { responses: true, chatCompletions: true, streaming: true } }));
    });
    upstream.on('connection', socket => {
      sockets.add(socket);
      socket.once('close', () => sockets.delete(socket));
    });
    context.after(async () => {
      try {
        if (service && !finished) { process.emit('SIGTERM'); await bounded(service); }
        for (const socket of sockets) socket.destroy();
        await new Promise(resolve => upstream.close(resolve));
        if (upstreamPort) await refused('127.0.0.1', upstreamPort);
        if (localPort) await refused(peerHost, localPort);
      } finally {
        for (const key of keys) original[key] === undefined ? delete process.env[key] : process.env[key] = original[key];
        await rm(root, { recursive: true, force: true });
      }
    });
    for (const key of keys) delete process.env[key];
    process.env.HOME = root;
    await new Promise((resolve, reject) => {
      upstream.once('error', reject);
      upstream.listen(0, '127.0.0.1', resolve);
    });
    upstreamPort = upstream.address().port;
    const config = join(root, 'profile.json');
    await writeFile(config, JSON.stringify({ remote: `http://127.0.0.1:${upstreamPort}`, host, port: 0 }), { mode: 0o600 });
    let announce;
    const ready = new Promise(resolve => { announce = resolve; });
    const output = [];
    service = serve(config, {
      stdout(text) {
        output.push(text);
        try { const value = JSON.parse(text); if (value.ready === true) announce(value); } catch {}
      },
      stderr: text => output.push(text),
    }, true).finally(() => { finished = true; });
    service.catch(() => {});
    const announcement = await bounded(Promise.race([ready, service.then(() => {
      throw new Error('serve stopped before reporting a listener');
    })]));
    localPort = announcement.listening.port;
    assert.equal(announcement.listening.host, host);
    assert.equal(await health(host, localPort), 200);
    process.emit('SIGTERM');
    assert.equal(await bounded(service), 0);
    await refused(peerHost, localPort);
    if (syntheticLookup) assert.ok(lookups > 0);
    assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => process.listenerCount(name)), beforeSignals);
    assert.equal(announcement.exposure, undefined, 'A numeric loopback listener is not exposed');
    assert.doesNotMatch(output.join(''), /non-loopback|Non-loopback/);
  });
}
