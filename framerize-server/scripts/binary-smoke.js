import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import http from 'node:http';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { gzipSync } from 'node:zlib';
import { artifactNames, currentTarget, digest } from './release-artifacts.js';
import { checkNativeAccount } from '../../.tools/scripts/native-account-smoke.js';

const [name, archiveName] = artifactNames(currentTarget());
const archive = path.resolve(process.argv[2] || `dist/${archiveName}`);
const directory = await mkdtemp(path.join(tmpdir(), 'framerize server binary '));
const serverToken = randomBytes(24).toString('hex');
const upstreamToken = randomBytes(24).toString('hex');
const wireBody = Buffer.from('{ "stream": true, "input": "fixture" }\r\n');
const disconnectBody = Buffer.from('{"stream":true,"input":"disconnect-fixture"}');
const streamBody = Buffer.from('data: {"delta":"fixture ✓"}\n\ndata: [DONE]\n\n');
const compressedBody = gzipSync(streamBody);
const calls = [];
let upstreamFailure;
let child;
let exit;
let logs = '';
let resolveDisconnected;
let upstreamClosed = false;
const disconnected = new Promise((resolve) => { resolveDisconnected = resolve; });

async function bounded(promise, message) {
  let timer;
  try { return await Promise.race([promise, new Promise((resolve, reject) => {
    timer = setTimeout(() => reject(new Error(message)), 5000);
  })]); } finally { clearTimeout(timer); }
}

const upstream = http.createServer(async (request, response) => {
  try {
    calls.push(request.url);
    assert.equal(request.headers.authorization, `Bearer ${upstreamToken}`);
    assert.equal(request.headers.cookie, undefined);
    assert.equal(request.headers['x-account-id'], undefined);
    if (request.url === '/v1/models') {
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end('{"data":[]}');
      return;
    }
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    if (Buffer.concat(chunks).equals(disconnectBody)) {
      response.once('close', () => { upstreamClosed = true; resolveDisconnected(); });
      response.writeHead(200, { 'content-type': 'text/event-stream' });
      response.write('data: pending\n\n');
      return;
    }
    assert.deepEqual(Buffer.concat(chunks), wireBody);
    const compressed = request.url === '/v1/chat/completions';
    const body = compressed ? compressedBody : streamBody;
    response.writeHead(200, { 'content-type': 'text/event-stream',
      ...(compressed ? { 'content-encoding': 'gzip' } : {}) });
    response.write(body.subarray(0, 9));
    setImmediate(() => response.end(body.subarray(9)));
  } catch (error) {
    upstreamFailure = error;
    response.writeHead(500);
    response.end();
  }
});

function request(port, route, token, body) {
  return new Promise((resolve, reject) => {
    const pending = http.request({ hostname: '127.0.0.1', port, path: route,
      method: body ? 'POST' : 'GET', headers: {
        ...(token ? { authorization: `Bearer ${token}` } : {}),
        'content-type': 'application/json', cookie: 'fixture=not-forwarded', 'x-account-id': 'not-forwarded',
      }, agent: false,
    }, (response) => {
      const chunks = [];
      response.on('data', (chunk) => chunks.push(chunk));
      response.on('error', reject);
      response.on('end', () => resolve({ status: response.statusCode, headers: response.headers,
        body: Buffer.concat(chunks) }));
    });
    pending.on('error', reject);
    pending.setTimeout(5000, () => pending.destroy(new Error('Fixture request timed out')));
    pending.end(body);
  });
}

function openStream(port) {
  return new Promise((resolve, reject) => {
    const pending = http.request({ hostname: '127.0.0.1', port, path: '/v1/responses', method: 'POST',
      headers: { authorization: `Bearer ${serverToken}`, 'content-type': 'application/json' }, agent: false,
    }, (response) => {
      response.once('error', reject);
      response.once('data', () => {
        if (response.statusCode !== 200) {
          response.destroy();
          reject(new Error('Unexpected cancellation fixture response'));
          return;
        }
        response.pause();
        resolve(response);
      });
    });
    pending.once('error', reject);
    pending.setTimeout(5000, () => pending.destroy(new Error('Cancellation fixture timed out')));
    pending.end(disconnectBody);
  });
}

const quote = (value) => `'${value.replaceAll("'", "''")}'`;
const environment = { PATH: process.platform === 'win32' ? process.env.SystemRoot : '',
  HOME: directory, USERPROFILE: directory, SystemRoot: process.env.SystemRoot,
  TMPDIR: directory, TEMP: directory, TMP: directory,
  FRAMERIZE_SERVER_TOKEN: serverToken, FIXTURE_UPSTREAM_TOKEN: upstreamToken };

try {
  if (process.platform === 'win32') {
    execFileSync('powershell', ['-NoLogo', '-NoProfile', '-Command',
      `Expand-Archive -LiteralPath ${quote(archive)} -DestinationPath ${quote(directory)} -Force`]);
  } else {
    execFileSync('tar', ['-xzf', archive, '-C', directory]);
  }
  assert.deepEqual((await readdir(directory)).sort(), ['LICENSE', 'NODE-LICENSE', name].sort());
  const executable = path.join(directory, name);
  for (const entry of ['LICENSE', 'NODE-LICENSE', name]) {
    assert.equal(digest(path.join(directory, entry)), digest(path.join(path.dirname(archive), entry)));
  }
  for (const license of ['LICENSE', 'NODE-LICENSE']) {
    const contents = await readFile(path.join(directory, license), 'utf8');
    assert.ok(contents.length > 0);
    assert.ok(!contents.includes('\r'));
  }
  const options = { cwd: directory, env: environment, encoding: 'utf8', timeout: 5000,
    stdio: ['ignore', 'pipe', 'pipe'] };
  assert.match(execFileSync(executable, ['--help'], options), /Usage: framerize \[/);
  assert.equal(execFileSync(executable, ['check'], options), '');
  await new Promise((resolve, reject) => {
    upstream.once('error', reject);
    upstream.listen(0, '127.0.0.1', resolve);
  });
  child = spawn(executable, ['serve', '--port', '0', '--host', '127.0.0.1',
    '--upstream', `http://127.0.0.1:${upstream.address().port}`, '--authorization-env', 'FIXTURE_UPSTREAM_TOKEN'],
  { cwd: directory, env: environment, stdio: ['ignore', 'pipe', 'pipe'] });
  exit = new Promise((resolve) => {
    child.once('exit', (code, signal) => resolve({ code, signal }));
    child.once('error', (error) => resolve({ error }));
  });
  child.stdout.resume();
  const port = await bounded(new Promise((resolve, reject) => {
    child.stderr.on('data', (chunk) => {
      logs += chunk.toString();
      for (const line of logs.split('\n').slice(0, -1)) {
        try {
          const record = JSON.parse(line);
          if (record.event === 'server.listening' && Number.isInteger(record.port)) resolve(record.port);
        } catch {}
      }
    });
    exit.then(() => reject(new Error('Fixture server exited before listening')));
  }), 'Fixture startup timed out');
  assert.deepEqual(JSON.parse((await request(port, '/health')).body), { status: 'ok', apiVersion: 1 });
  assert.equal((await request(port, '/ready')).status, 401);
  assert.equal((await request(port, '/v1/responses', 'wrong-token', wireBody)).status, 401);
  assert.deepEqual(calls, []);
  const ready = await request(port, '/ready', serverToken);
  assert.equal(ready.status, 200);
  assert.deepEqual(JSON.parse(ready.body), { apiVersion: 1, status: 'ready',
    capabilities: { responses: true, chatCompletions: true, streaming: true } });
  for (const route of ['/v1/responses', '/v1/chat/completions']) {
    const response = await request(port, route, serverToken, wireBody);
    assert.equal(response.status, 200);
    const compressed = route === '/v1/chat/completions';
    assert.equal(response.headers['content-encoding'], compressed ? 'gzip' : undefined);
    assert.deepEqual(response.body, compressed ? compressedBody : streamBody);
  }
  assert.ifError(upstreamFailure);
  assert.deepEqual(calls, ['/v1/models', '/v1/responses', '/v1/chat/completions']);
  const active = await bounded(openStream(port), 'Cancellation stream startup timed out');
  assert.equal((await request(port, '/health')).status, 200);
  assert.equal(upstreamClosed, false, 'Upstream closed before client disconnect');
  assert.equal(active.destroyed, false, 'Downstream closed before client disconnect');
  assert.equal(active.complete, false, 'Stream completed before client disconnect');
  active.destroy();
  await bounded(disconnected, 'Client disconnect did not close upstream');
  assert.ifError(upstreamFailure);
  assert.equal(calls.length, 4);
  assert.ok(!logs.includes(serverToken) && !logs.includes(upstreamToken));
  assert.ok(!logs.includes(wireBody.toString()) && !logs.includes('not-forwarded') &&
    !logs.includes('disconnect-fixture'));
  child.kill('SIGTERM');
  const stopped = await bounded(exit, 'Fixture shutdown timed out');
  if (process.platform !== 'win32') assert.deepEqual(stopped, { code: 0, signal: null });
  process.stdout.write(`${JSON.stringify(await checkNativeAccount(executable))}\n`);
  process.stdout.write('Extracted binary: help, config, auth, readiness, streams, gzip, disconnect and cleanup passed\n');
  if (process.platform === 'win32') process.stdout.write('Windows termination checked; POSIX graceful signal assertion not applicable\n');
} finally {
  try {
    if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    if (exit) await bounded(exit, 'Fixture process cleanup timed out');
  } finally {
    upstream.closeAllConnections();
    await new Promise((resolve) => upstream.close(resolve));
    await rm(directory, { recursive: true, force: true });
  }
}
