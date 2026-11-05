import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, readdir, rm } from 'node:fs/promises';
import { createServer, request } from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

async function bounded(pending, message) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(message)), 5000);
    })]);
  } finally { clearTimeout(timer); }
}

function exchange(port, path, body) {
  return new Promise((resolve, reject) => {
    const outgoing = request({ host: '127.0.0.1', port, path, agent: false,
      method: body === undefined ? 'GET' : 'POST', signal: AbortSignal.timeout(2000),
      headers: body === undefined ? {} : { 'content-type': 'application/json' },
    }, response => {
      let received = '';
      response.setEncoding('utf8').on('data', chunk => { received += chunk; });
      response.on('error', reject);
      response.on('end', () => resolve({ status: response.statusCode, body: received }));
    });
    outgoing.on('error', reject);
    outgoing.end(body);
  });
}

test('real launcher serves trusted HTTP traffic using only environment configuration',
  { timeout: 20_000 }, async context => {
    const packageRoot = resolve(process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
      ?? fileURLToPath(new URL('../', import.meta.url)));
    const launcher = join(packageRoot, 'bin/framerize-server.js');
    const root = await mkdtemp(join(tmpdir(), 'server-trusted-env-'));
    const input = JSON.stringify({ model: 'synthetic-model', stream: false,
      input: 'synthetic-input-canary' });
    const completed = JSON.stringify({ id: 'resp_synthetic', object: 'response',
      status: 'completed', model: 'synthetic-model', output: [{ type: 'message',
        role: 'assistant', content: [{ type: 'output_text', text: 'synthetic-output-canary' }] }] });
    const calls = [];
    const upstream = createServer((incoming, outgoing) => {
      let body = '';
      incoming.setEncoding('utf8').on('data', chunk => { body += chunk; });
      incoming.on('error', () => outgoing.destroy());
      incoming.on('end', () => {
        calls.push({ method: incoming.method, path: incoming.url, body,
          authorization: incoming.headers.authorization });
        const models = incoming.method === 'GET' && incoming.url === '/v1/models';
        const inference = incoming.method === 'POST' && incoming.url === '/v1/responses';
        outgoing.writeHead(models || inference ? 200 : 404, { 'content-type': 'application/json' });
        outgoing.end(models ? '{"object":"list","data":[{"id":"synthetic-model"}]}'
          : inference ? completed : '{"error":"unexpected_fixture_route"}');
      });
    });
    let child;
    let closed;
    let lines;
    let upstreamClosed;
    const closeUpstream = () => {
      upstream.closeAllConnections();
      return upstreamClosed ??= new Promise(resolve => upstream.close(resolve));
    };
    context.after(async () => {
      try {
        if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
        if (closed) await bounded(closed, 'Server child cleanup timed out');
      } finally {
        lines?.close();
        try { await bounded(closeUpstream(), 'Upstream fixture cleanup timed out'); }
        finally { await rm(root, { recursive: true, force: true }); }
      }
    });
    assert.deepEqual(await readdir(root), []);
    const opened = once(upstream, 'listening');
    upstream.listen(0, '127.0.0.1');
    await bounded(opened, 'Upstream fixture did not start');
    const upstreamPort = upstream.address().port;
    child = spawn(process.execPath, [launcher, 'serve'], {
      cwd: root, stdio: ['ignore', 'pipe', 'pipe'],
      env: { HOME: root, USERPROFILE: root, XDG_CONFIG_HOME: root,
        TMPDIR: root, TEMP: root, TMP: root, NODE_DISABLE_COMPILE_CACHE: '1',
        FRAMERIZE_SERVER_BACKEND: 'http', FRAMERIZE_SERVER_HOST: '127.0.0.1',
        FRAMERIZE_SERVER_PORT: '0', FRAMERIZE_SERVER_UPSTREAM: `http://127.0.0.1:${upstreamPort}`,
        ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}),
      },
    });
    closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })));
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8').on('data', chunk => { stdout += chunk; });
    child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk; });
    lines = createInterface({ input: child.stderr });
    const listening = new Promise((resolve, reject) => {
      child.once('error', () => reject(new Error('Server launcher could not start')));
      child.once('close', (code, signal) => reject(new Error(
        `Server exited before listening: code=${code}, signal=${signal}`)));
      lines.on('line', line => {
        let record;
        try { record = JSON.parse(line); }
        catch { reject(new Error('Server logs must be JSON')); return; }
        if (record?.event === 'server.listening') resolve(record.port);
      });
    });
    const port = await bounded(listening, 'Server did not report a listener');
    assert.ok(Number.isInteger(port) && port > 0 && port <= 65535, 'Expected an assigned port');
    assert.notEqual(port, upstreamPort);
    const ready = await exchange(port, '/ready');
    const response = await exchange(port, '/v1/responses', input);
    assert.equal(child.exitCode, null);
    assert.equal(child.signalCode, null);
    assert.equal(child.kill('SIGTERM'), true);
    const stopped = await bounded(closed, 'Server did not stop after SIGTERM');
    if (process.platform === 'win32') assert.ok(stopped.code === 0 || stopped.signal === 'SIGTERM');
    else assert.deepEqual(stopped, { code: 0, signal: null });
    await assert.rejects(exchange(port, '/health'), { code: 'ECONNREFUSED' });
    await bounded(closeUpstream(), 'Upstream fixture did not stop');
    await assert.rejects(exchange(upstreamPort, '/health'), { code: 'ECONNREFUSED' });
    assert.deepEqual(await readdir(root), [], 'Trusted startup must not write credential or profile files');
    assert.equal(stdout, '');
    assert.doesNotMatch(stderr, /server\.failed|synthetic-(?:input|output)-canary/);
    assert.equal(stderr.includes(root), false, 'Server logs must not include the private state path');
    context.diagnostic(`Listener started; /ready=${ready.status}; /v1/responses=${response.status}; `
      + `upstream calls=${calls.length}; child and fixture closed; both ports refused connections.`);
    assert.equal(ready.status, 200, 'Trusted default must permit unauthenticated readiness');
    assert.equal(JSON.parse(ready.body).status, 'ready');
    assert.equal(response.status, 200, 'Trusted default must permit unauthenticated inference');
    assert.equal(response.body, completed);
    assert.deepEqual(calls, [
      { method: 'GET', path: '/v1/models', body: '', authorization: undefined },
      { method: 'POST', path: '/v1/responses', body: input, authorization: undefined },
    ]);
  });
