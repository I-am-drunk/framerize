import assert from 'node:assert/strict';
import { EventEmitter, once } from 'node:events';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer, request } from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = resolve(process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const { loadConfig } = await import(pathToFileURL(join(root, 'dist/config.js')).href);
const { captureSettings } = await import(pathToFileURL(join(root, 'dist/settings.js')).href);
const { run } = await import(pathToFileURL(join(root, 'dist/cli.js')).href);
const key = 'FRAMERIZE_TEST_SAVED_UPSTREAM_TOKEN';

async function bounded(pending) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Saved credential fixture timed out')), 5000);
    })]);
  } finally { clearTimeout(timer); }
}

function exchange(port, path, body) {
  return new Promise((resolve, reject) => {
    const outgoing = request({ host: '127.0.0.1', port, path, agent: false,
      method: body === undefined ? 'GET' : 'POST', signal: AbortSignal.timeout(2000),
      headers: { 'content-type': 'application/json', authorization: 'Bearer local-only' },
    }, incoming => {
      let text = '';
      incoming.setEncoding('utf8').on('data', chunk => { text += chunk; });
      incoming.on('error', reject).on('end', () => resolve({ status: incoming.statusCode, body: text }));
    });
    outgoing.on('error', reject).end(body);
  });
}

for (const [name, env, expected] of [
  ['saved environment', {}, 'saved-synthetic-token'],
  ['environment override', { [key]: 'override-synthetic-token' }, 'override-synthetic-token'],
  ['empty override', { [key]: '' }, undefined],
]) test(`HTTP service restart uses the ${name}`, { timeout: 20_000 }, async t => {
  const home = await mkdtemp(join(tmpdir(), 'server-saved-credential-'));
  const calls = [];
  const input = JSON.stringify({ model: 'synthetic-model', input: 'hello', stream: false });
  const output = JSON.stringify({ id: 'resp_synthetic', output: [], status: 'completed' });
  const upstream = createServer((incoming, outgoing) => {
    let body = '';
    incoming.setEncoding('utf8').on('data', chunk => { body += chunk; });
    incoming.on('error', () => outgoing.destroy());
    incoming.on('end', () => {
      calls.push({ path: incoming.url, authorization: incoming.headers.authorization, body });
      outgoing.writeHead(200, { 'content-type': 'application/json' });
      outgoing.end(incoming.url === '/v1/models' ? '{"object":"list","data":[]}' : output);
    });
  });
  t.after(async () => {
    upstream.closeAllConnections();
    await bounded(new Promise(resolve => upstream.close(resolve)));
    await rm(home, { recursive: true, force: true });
  });
  const opened = once(upstream, 'listening');
  upstream.listen(0, '127.0.0.1');
  await bounded(opened);
  const upstreamPort = upstream.address().port;
  const sourceEnv = { [key]: 'saved-synthetic-token' };
  const config = loadConfig({ upstream: `http://127.0.0.1:${upstreamPort}`, port: 0,
    authorizationEnv: key }, sourceEnv);
  const path = join(home, 'server.json');
  await writeFile(path, JSON.stringify(captureSettings(config, sourceEnv)), { mode: 0o600 });
  const savedBytes = await readFile(path, 'utf8');
  const processValue = process.env[key];
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const events = new EventEmitter();
    events.stdout = events.stderr = { write() {} };
    let ready, response, port;
    let announce, fail;
    const listening = new Promise((resolve, reject) => { announce = resolve; fail = reject; });
    const running = run(['serve', '--config', path], { env, process: events,
      logger: { info(event, details) { if (event === 'server.listening') announce(details.port); } },
    });
    running.catch(fail);
    try {
      port = await bounded(listening);
      ready = await exchange(port, '/ready');
      response = await exchange(port, '/v1/responses', input);
    } finally {
      events.emit('SIGTERM');
      assert.equal(await bounded(running), 0);
    }
    await assert.rejects(exchange(port, '/health'), { code: 'ECONNREFUSED' });
    assert.equal(events.listenerCount('SIGTERM'), 0);
    assert.equal(events.listenerCount('SIGINT'), 0);
    assert.equal(ready.status, expected ? 200 : 503, 'Readiness must use the resolved environment');
    assert.equal(response.status, expected ? 200 : 503);
    if (expected) assert.equal(response.body, output);
    else assert.equal(JSON.parse(response.body).error.code, 'credentials_unavailable');
  }
  assert.deepEqual(calls, expected ? Array.from({ length: 2 }, () => [
    { path: '/v1/models', authorization: `Bearer ${expected}`, body: '' },
    { path: '/v1/responses', authorization: `Bearer ${expected}`, body: input },
  ]).flat() : []);
  assert.equal(await readFile(path, 'utf8'), savedBytes);
  assert.equal(process.env[key], processValue, 'Saved credentials must not alter process.env');
  upstream.closeAllConnections();
  await bounded(new Promise(resolve => upstream.close(resolve)));
  await assert.rejects(exchange(upstreamPort, '/health'), { code: 'ECONNREFUSED' });
});
