// Managed-startup readiness: the listener must be the service's own process.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer, request } from 'node:http';
import { createServer as createTcpServer } from 'node:net';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const source = name => JSON.stringify(fileURLToPath(new URL(`../src/${name}.ts`, import.meta.url)));
const compiled = await build({ stdin: { contents: `export * from ${source('serviceReadiness')};`,
  resolveDir: fileURLToPath(new URL('.', import.meta.url)) },
alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) },
bundle: true, platform: 'node', format: 'esm', write: false });
const ready = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

/** A local HTTP server that the test owns and closes; the handler decides the answer. */
async function listen(context, handler) {
  const server = createServer(handler);
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  context.after(() => new Promise(resolve => { server.closeAllConnections(); server.close(resolve); }));
  return server.address().port;
}

const running = pid => async () => ({ running: true, pid });

test('an unrelated /health or /ready server on the port is never the managed listener', async context => {
  for (const handler of [(request, response) => response.end('{"status":"ok"}'),
    (request, response) => { response.writeHead(200, { 'x-framerize-remote-pid': '999999' }); response.end(); },
    (request, response) => { response.writeHead(request.url === '/ready' ? 200 : 404); response.end(); }]) {
    const port = await listen(context, handler);
    await assert.rejects(ready.waitForServiceListener({ host: '127.0.0.1', port, service: running(4242), timeoutMs: 600 }),
      { code: 'service_not_ready' });
  }
});

test('the preflight reports an occupied port and leaves that listener running', async context => {
  const port = await listen(context, (request, response) => response.end('unrelated'));
  await assert.rejects(ready.checkLocalPort('127.0.0.1', port), { code: 'service_port_busy' });
  const answer = await new Promise((resolve, reject) => request({ host: '127.0.0.1', port, agent: false },
    response => { let body = ''; response.on('data', chunk => { body += chunk; }); response.on('end', () => resolve(body)); })
    .on('error', reject).end());
  assert.equal(answer, 'unrelated');
});

test('a stopped unit fails at once rather than waiting for the deadline', async context => {
  const port = await listen(context, (request, response) => response.end());
  const started = Date.now();
  await assert.rejects(ready.waitForServiceListener({ host: '127.0.0.1', port,
    service: async () => ({ running: false, pid: undefined }), timeoutMs: 5000 }), { code: 'service_failed' });
  assert.ok(Date.now() - started < 2000);
});

/** A raw TCP peer that writes `reply` and then holds the socket open or closes it. */
async function raw(context, reply, { hold = false } = {}) {
  const sockets = new Set();
  const server = createTcpServer(socket => {
    sockets.add(socket);
    socket.on('error', () => {});
    socket.once('data', () => { socket.write(reply); if (!hold) socket.destroy(); });
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  context.after(() => new Promise(resolve => { for (const socket of sockets) socket.destroy(); server.close(resolve); }));
  return server.address().port;
}

const headers = pid => `HTTP/1.1 200 OK\r\nx-framerize-remote-pid: ${pid}\r\nContent-Length: 20\r\n\r\n`;

test('a truncated, stalled or oversized /health reply with the right PID is not ready', async context => {
  const errors = [];
  const onError = error => errors.push(error);
  process.on('uncaughtException', onError);
  context.after(() => process.off('uncaughtException', onError));
  for (const [reply, hold] of [[`${headers(4242)}{"status":`, false], [`${headers(4242)}{"st`, true],
    [`HTTP/1.1 200 OK\r\nx-framerize-remote-pid: 4242\r\nContent-Length: 9000\r\n\r\n${'x'.repeat(9000)}`, false]]) {
    const port = await raw(context, reply, { hold });
    const started = Date.now();
    await assert.rejects(ready.waitForServiceListener({ host: '127.0.0.1', port, service: running(4242), timeoutMs: 2500 }),
      { code: 'service_not_ready' });
    assert.ok(Date.now() - started < 4500);
  }
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(errors, []);
});

test('a complete /health from the service PID is ready, and a caller abort stops the wait', async context => {
  const port = await raw(context, `${headers(4242)}{"status":"ok"}     `);
  assert.equal(await ready.waitForServiceListener({ host: '127.0.0.1', port, service: running(4242), timeoutMs: 2000 }), 4242);
  const controller = new AbortController();
  const wait = ready.waitForServiceListener({ host: '127.0.0.1', port, service: running(1), timeoutMs: 10_000,
    signal: controller.signal });
  setTimeout(() => controller.abort(), 100);
  await assert.rejects(wait, { code: 'aborted' });
});

/** Bundles the source CLI into a private temp file, so the child never reads the shared dist. */
async function privateLauncher(context) {
  const { mkdtemp, rm, writeFile } = await import('node:fs/promises');
  const { tmpdir } = await import('node:os');
  const { join } = await import('node:path');
  const root = await mkdtemp(join(tmpdir(), 'remote-frm-child-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  const out = await build({ stdin: { contents: `import { runFrm } from ${source('frmCli')};
runFrm(process.argv.slice(2), { stdout: t => process.stdout.write(t), stderr: t => process.stderr.write(t) },
  { program: [process.execPath, process.argv[1]] }).then(code => { process.exitCode = code; });`,
  resolveDir: fileURLToPath(new URL('.', import.meta.url)) },
  alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) },
  bundle: true, platform: 'node', format: 'esm', write: false });
  const path = join(root, 'frm.mjs');
  await writeFile(path, out.outputFiles[0].contents);
  return { root, path };
}

test('a captured real frm serve child is recognized by its own PID and released on stop', { timeout: 20_000 }, async context => {
  const { spawn } = await import('node:child_process');
  const { join } = await import('node:path');
  const remote = await listen(context, (request, response) => {
    response.writeHead(request.url === '/ready' ? 200 : 404, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ apiVersion: 1, status: 'ready',
      capabilities: { responses: true, chatCompletions: true, streaming: true } }));
  });
  const probe = createTcpServer();
  await new Promise(resolve => probe.listen(0, '127.0.0.1', resolve));
  const port = probe.address().port;
  await new Promise(resolve => probe.close(resolve));
  const { root, path } = await privateLauncher(context);
  const child = spawn(process.execPath, [path, 'serve'], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'],
    env: { HOME: root, FRM_HOME: join(root, 'home'), FRM_REMOTE: `http://127.0.0.1:${remote}`, FRM_PORT: String(port) } });
  const closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })));
  context.after(async () => { if (child.exitCode === null) child.kill('SIGKILL'); await closed; });
  await assert.rejects(ready.waitForServiceListener({ host: '127.0.0.1', port, service: running(child.pid + 1), timeoutMs: 3000 }),
    { code: 'service_not_ready' });
  assert.equal(await ready.waitForServiceListener({ host: '127.0.0.1', port, service: running(child.pid), timeoutMs: 8000 }), child.pid);
  child.kill('SIGTERM');
  assert.deepEqual(await closed, { code: 0, signal: null });
  await ready.checkLocalPort('127.0.0.1', port);
});
