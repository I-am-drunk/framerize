import test from 'node:test';
import assert from 'node:assert/strict';
import dns from 'node:dns';
import { createServer, request } from 'node:http';
import { mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/index.ts', import.meta.url))],
  alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) },
  bundle: true, platform: 'node', format: 'esm', write: false });
const { runFrm } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

function post(port, headers = {}) {
  return new Promise((resolve, reject) => {
    const client = request({ host: '127.0.0.1', port, path: '/v1/responses', method: 'POST', agent: false,
      headers: { 'content-type': 'application/json', ...headers } }, response => {
      let body = '';
      response.setEncoding('utf8').on('data', chunk => { body += chunk; });
      response.on('end', () => resolve({ status: response.statusCode, body }));
    });
    client.on('error', reject);
    client.end('{"model":"synthetic","stream":true}');
  });
}

async function serveFromEnvironment(context, env, remoteHost) {
  const root = await mkdtemp(join(tmpdir(), 'remote-frm-env-'));
  const calls = [];
  const remote = createServer((incoming, response) => {
    calls.push({ path: incoming.url, authorization: incoming.headers.authorization, host: incoming.headers.host });
    incoming.resume();
    if (incoming.url === '/ready') {
      response.setHeader('content-type', 'application/json');
      response.end(JSON.stringify({ apiVersion: 1, status: 'ready',
        capabilities: { responses: true, chatCompletions: true, streaming: true } }));
      return;
    }
    response.writeHead(200, { 'content-type': 'text/event-stream' });
    response.end('data: {"type":"response.output_text.delta","delta":"OK"}\n\ndata: [DONE]\n\n');
  });
  await new Promise(resolve => remote.listen(0, '127.0.0.1', resolve));
  const saved = { ...process.env };
  context.after(async () => {
    for (const name of Object.keys(process.env)) if (!(name in saved)) delete process.env[name];
    Object.assign(process.env, saved);
    await new Promise(resolve => remote.close(resolve));
    await rm(root, { recursive: true, force: true });
  });
  Object.assign(process.env, { FRM_HOME: join(root, 'profile'), FRM_PORT: '0',
    FRM_REMOTE: `http://${remoteHost}:${remote.address().port}`, ...env });
  let output = '';
  let started;
  const listening = new Promise(resolve => { started = resolve; });
  const serving = runFrm(['serve', '--json'], { stderr() {}, stdout(text) {
    output += text;
    const event = JSON.parse(text);
    if (event.listening) started(event.listening.port);
  } }, { program: ['/usr/bin/node', '/synthetic/frm.js'] });
  const port = await Promise.race([listening, serving.then(() => { throw new Error('frm ended early'); })]);
  return { root, calls, port, serving, output: () => output };
}

test('FRM_* serve forwards a streamed request without tokens or saved files', { timeout: 10_000 }, async context => {
  const { root, calls, port, serving } = await serveFromEnvironment(context, {}, '127.0.0.1');
  const reply = await post(port);
  assert.equal(reply.status, 200);
  assert.match(reply.body, /"delta":"OK"/);
  process.emit('SIGTERM');
  assert.equal(await serving, 0);
  assert.deepEqual(calls.map(call => [call.path, call.authorization]),
    [['/ready', undefined], ['/ready', undefined], ['/v1/responses', undefined]]);
  const entries = await readdir(root, { recursive: true, withFileTypes: true });
  assert.ok(entries.every(entry => entry.isDirectory()));
});

test('a Tailscale MagicDNS remote works over HTTP with optional environment tokens', { timeout: 10_000 }, async context => {
  const lookup = dns.lookup;
  context.mock.method(dns, 'lookup', (host, options, callback) =>
    lookup(host === 'desktop.tail1234.ts.net' ? '127.0.0.1' : host, options, callback));
  const { calls, port, serving } = await serveFromEnvironment(context,
    { FRM_SERVER_TOKEN: 'synthetic-server', FRM_LOCAL_TOKEN: 'synthetic-local' }, 'desktop.tail1234.ts.net');
  assert.equal((await post(port)).status, 401);
  const reply = await post(port, { authorization: 'Bearer synthetic-local' });
  assert.equal(reply.status, 200);
  assert.match(reply.body, /"delta":"OK"/);
  process.emit('SIGTERM');
  assert.equal(await serving, 0);
  assert.deepEqual(calls.map(call => call.path), ['/ready', '/ready', '/v1/responses']);
  assert.ok(calls.every(call => call.authorization === 'Bearer synthetic-server'
    && call.host.startsWith('desktop.tail1234.ts.net:')));
});

test('noninteractive frm and frm setup report a missing server address deterministically', async context => {
  const root = await mkdtemp(join(tmpdir(), 'remote-frm-missing-'));
  const saved = { ...process.env };
  context.after(async () => {
    for (const name of Object.keys(process.env)) if (!(name in saved)) delete process.env[name];
    Object.assign(process.env, saved);
    await rm(root, { recursive: true, force: true });
  });
  delete process.env.FRM_REMOTE;
  process.env.FRM_HOME = join(root, 'profile');
  const io = { stdout() { assert.fail('no output expected'); }, stderr() {} };
  for (const argv of [[], ['setup']]) {
    await assert.rejects(runFrm(argv, io, { program: ['/synthetic/frm'] }), { code: 'remote_required' });
  }
  assert.deepEqual(await readdir(root), []);
});

test('frm setup asks for the missing address before offering optional client setup', async context => {
  const root = await mkdtemp(join(tmpdir(), 'remote-frm-ask-'));
  const saved = { ...process.env };
  context.after(async () => {
    for (const name of Object.keys(process.env)) if (!(name in saved)) delete process.env[name];
    Object.assign(process.env, saved);
    await rm(root, { recursive: true, force: true });
  });
  delete process.env.FRM_REMOTE;
  process.env.FRM_HOME = join(root, 'profile');
  const questions = [];
  let output = '';
  assert.equal(await runFrm(['setup'], { stderr() {}, stdout(text) { output += text; },
    async ask(question) {
      questions.push(question);
      return questions.length === 1 ? ' http://100.64.0.10:8787 ' : 'no';
    } },
  { program: ['/synthetic/frm'] }), 0);
  assert.equal(questions.length, 2);
  assert.match(questions[0], /server/i);
  assert.match(questions[1], /Claude Code.*Codex/);
  assert.match(output, /^Profile saved\.\n/);
  const { readFile } = await import('node:fs/promises');
  const profile = JSON.parse(await readFile(join(root, 'profile', 'framerize-remote.json'), 'utf8'));
  assert.equal(profile.remote, 'http://100.64.0.10:8787');
  assert.equal(profile.serverToken, undefined); assert.equal(profile.localToken, undefined);
});
