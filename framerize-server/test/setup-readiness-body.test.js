import assert from 'node:assert/strict';
import { once } from 'node:events';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer } from 'node:http';
import { connect } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { gzipSync } from 'node:zlib';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/setup.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
  external: ['./backend.js'], plugins: [{ name: 'source-core', setup(builder) {
    builder.onResolve({ filter: /\/dist\/core\.js$/ }, () => ({
      path: fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)),
    }));
  } }],
});
const { runSetupCommand } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);
const limit = 64 * 1024;
const ready = JSON.stringify({ status: 'ready', apiVersion: 1,
  capabilities: { responses: true, chatCompletions: true, streaming: true } });

async function within(pending, milliseconds, label) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(`${label} did not settle`)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function refused(port) {
  const socket = connect({ host: '127.0.0.1', port });
  try {
    const result = await within(new Promise(resolve => {
      socket.once('error', error => resolve(error.code));
      socket.once('connect', () => resolve('connected'));
    }), 1000, 'Port check');
    assert.equal(result, 'ECONNREFUSED');
  } finally { socket.destroy(); }
}

async function fixture(context, respond) {
  const root = await mkdtemp(join(tmpdir(), 'setup-ready-body-'));
  const controller = new AbortController();
  const state = { requests: [], output: '', firstClosed: false };
  const sockets = new Set();
  let pending, port;
  const server = createServer((request, response) => {
    state.requests.push({ method: request.method, path: request.url });
    response.setHeader('x-framerize-server-pid', String(process.pid));
    response.setHeader('content-type', 'application/json');
    if (state.requests.length === 1) {
      response.once('close', () => { state.firstClosed = true; });
      respond(response);
    } else response.end(ready);
  });
  server.on('connection', socket => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
  });
  context.after(async () => {
    controller.abort(new Error('Fixture cleanup'));
    try {
      if (pending) await within(pending, 3000, 'Setup cleanup');
    } finally {
      for (const socket of sockets) socket.destroy();
      try {
        await within(new Promise(resolve => server.close(resolve)), 1000, 'Listener cleanup');
        if (port) await refused(port);
        context.diagnostic(JSON.stringify({ cleanup: 'closed', portRefused: Boolean(port) }));
      } finally { await rm(root, { recursive: true, force: true }); }
    }
  });
  server.listen(0, '127.0.0.1');
  await within(once(server, 'listening'), 1000, 'Listener startup');
  port = server.address().port;
  const origin = `http://127.0.0.1:${port}`;
  const invoke = () => {
    pending = runSetupCommand(['pair', '--backend', 'http', '--host', '127.0.0.1',
      '--port', String(port), '--json'], {
      env: { HOME: root, FRAMERIZE_SERVER_HOME: join(root, 'server') },
      platform: 'linux', uid: 1000, program: [process.execPath, '/synthetic/server'],
      signal: controller.signal, stdout: { write(text) { state.output += text; } },
      run: async () => { throw new Error('No service or Tailscale command is permitted'); },
    }).then(value => ({ value }), error => ({ error }));
    return pending;
  };
  return { state, origin, controller, invoke };
}

for (const scenario of [
  { name: 'ordinary JSON', size: ready.length, requests: 1 },
  { name: 'exactly 64 KiB', size: limit, requests: 1 },
  { name: 'more than 64 KiB', size: limit + 1, requests: 2 },
  { name: 'gzip expanding beyond 64 KiB', size: limit + 1, gzip: true, requests: 2 },
]) {
  test(`pair bounds readiness JSON: ${scenario.name}`, { timeout: 6000 }, async context => {
    const body = Buffer.from(ready.padEnd(scenario.size, ' '));
    const { state, origin, invoke } = await fixture(context, response => {
      if (scenario.gzip) response.setHeader('content-encoding', 'gzip');
      response.end(scenario.gzip ? gzipSync(body) : body);
    });
    const outcome = await within(invoke(), 3500, 'Pairing');
    assert.ifError(outcome.error);
    assert.equal(outcome.value, 0);
    context.diagnostic(JSON.stringify({ bytes: body.length, requests: state.requests.length }));
    assert.equal(state.requests.length, scenario.requests,
      'Oversized readiness must be discarded before the next valid reply is accepted');
    assert.ok(state.requests.every(request => request.method === 'GET' && request.path === '/ready'));
    assert.deepEqual(JSON.parse(state.output), { origin, pairingUrl: `${origin}/pair`, ready: true });
  });
}

test('pair cancels an oversized unfinished body before its request timeout', {
  timeout: 6000,
}, async context => {
  const { state, invoke } = await fixture(context, response => {
    response.write(ready.padEnd(limit + 1, ' '));
  });
  const outcome = await within(invoke(), 1500, 'Oversized body cancellation and retry');
  assert.ifError(outcome.error);
  assert.equal(outcome.value, 0);
  assert.equal(state.requests.length, 2);
  assert.equal(state.firstClosed, true, 'The oversized response must close before pairing completes');
});

test('pair preserves the cancellation reason while reading a readiness body', {
  timeout: 6000,
}, async context => {
  let started;
  const received = new Promise(resolve => { started = resolve; });
  const { state, controller, invoke } = await fixture(context, response => {
    response.write('{');
    started();
  });
  const pending = invoke();
  await within(received, 1000, 'Readiness request');
  const reason = new Error('Synthetic readiness cancellation');
  controller.abort(reason);
  const outcome = await within(pending, 1000, 'Cancelled pairing');
  assert.equal(outcome.error, reason);
  assert.equal(state.output, '');
  assert.equal(state.requests.length, 1);
});
