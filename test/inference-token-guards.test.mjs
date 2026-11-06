import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { once, getEventListeners } from 'node:events';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const entry = new URL('../scripts/check-inference.ts', import.meta.url);
const bundle = await build({ stdin: {
  contents: 'export * from "./check-inference.ts"; export * from "./validation/inference.ts";',
  resolveDir: fileURLToPath(new URL('../scripts/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false,
  define: { 'import.meta.url': JSON.stringify(entry.href) } });
const { runInferenceCheck, validateInferenceOptions } = await import(
  `data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].contents).toString('base64')}`);
const options = { endpoint: 'http://127.0.0.1:1', model: 'synthetic-model' };
const args = ['--endpoint', options.endpoint, '--model', options.model];

test('optional tokens normalize to null while defined invalid tokens fail', () => {
  assert.equal(validateInferenceOptions(options).token, null);
  assert.equal(validateInferenceOptions({ ...options, token: null }).token, null);
  assert.equal(validateInferenceOptions({ ...options, token: 'synthetic-token' }).token, 'synthetic-token');
  for (const token of ['', 'bad token', 'bad\n', 'a'.repeat(8193), false, 1, {}]) {
    assert.throws(() => validateInferenceOptions({ ...options, token }));
  }
});

test('consent, reference syntax and pre-abort guards precede credential lookup', async () => {
  let reads = 0;
  const env = new Proxy({}, { get() { reads += 1; throw new Error('unexpected lookup'); } });
  const signal = AbortSignal.abort();
  const invalid = [[], ['--send-one-request', '--token-env', 'BAD-NAME'],
    ['--send-one-request', '--token-env', 'TOKEN', '--token-file', 'synthetic-file'],
    ['--send-one-request', '--token-env', ''], ['--send-one-request', '--token-file', '']];
  for (const extra of invalid) {
    const result = await runInferenceCheck([...args, ...extra], env, signal);
    assert.equal(result.code, 'configuration');
    assert.equal(result.requestAttempts, 0);
  }
  for (const extra of [[], ['--token-env', 'TOKEN']]) {
    const result = await runInferenceCheck([...args, '--send-one-request', ...extra], env, signal);
    assert.equal(result.code, 'aborted');
    assert.equal(result.requestAttempts, 0);
  }
  assert.equal(reads, 0);
  assert.equal(getEventListeners(signal, 'abort').length, 0);
});

test('explicit file and environment credentials never fall back to tokenless', { timeout: 5000 }, async () => {
  const root = await mkdtemp(join(tmpdir(), 'inference-token-guards-'));
  const path = join(root, 'token');
  const controller = new AbortController();
  const sockets = new Set();
  const closed = [];
  const requests = [];
  const server = createServer((request, response) => {
    requests.push(request.headers.authorization);
    request.resume();
    response.writeHead(200, { 'content-type': 'text/event-stream' });
    response.end('data: {"choices":[{"index":0,"delta":{"content":"OK"},"finish_reason":"stop"}]}\n\ndata: [DONE]\n\n');
  });
  server.on('connection', socket => {
    sockets.add(socket);
    closed.push(new Promise(resolve => socket.once('close', () => { sockets.delete(socket); resolve(); })));
  });
  try {
    server.listen(0, '127.0.0.1');
    await once(server, 'listening');
    const argv = ['--endpoint', `http://127.0.0.1:${server.address().port}`,
      '--model', options.model, '--send-one-request', '--timeout-ms', '1000'];
    const missing = await runInferenceCheck([...argv, '--token-file', path], {}, controller.signal);
    assert.equal(missing.code, 'credential');
    assert.equal(missing.requestAttempts, 0);
    for (const token of ['', 'bad token', 'a'.repeat(8193)]) {
      await writeFile(path, token, { mode: 0o600 });
      for (const [reference, env] of [[['--token-file', path], {}], [['--token-env', 'TOKEN'], { TOKEN: token }]]) {
        const result = await runInferenceCheck([...argv, ...reference], env, controller.signal);
        assert.equal(result.code, 'credential');
        assert.equal(result.requestAttempts, 0);
      }
      assert.equal(await readFile(path, 'utf8'), token);
    }
    assert.deepEqual(requests, []);
    await writeFile(path, 'synthetic-file-token\n', { mode: 0o600 });
    const result = await runInferenceCheck([...argv, '--token-file', path], {}, controller.signal);
    assert.equal(result.ok, true);
    assert.equal(result.requestAttempts, 1);
    assert.deepEqual(requests, ['Bearer synthetic-file-token']);
    await Promise.all(closed);
    assert.equal(sockets.size, 0);
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    assert.equal(await readFile(path, 'utf8'), 'synthetic-file-token\n');
  } finally {
    controller.abort();
    for (const socket of sockets) socket.destroy();
    if (server.listening) await new Promise(resolve => server.close(resolve));
    await Promise.all(closed);
    await rm(root, { recursive: true, force: true });
  }
  await assert.rejects(readFile(path), { code: 'ENOENT' });
});
