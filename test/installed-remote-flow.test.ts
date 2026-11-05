import assert from 'node:assert/strict';
import test from 'node:test';
import { execFile, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { once } from 'node:events';
import type { AddressInfo } from 'node:net';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { promisify } from 'node:util';
import { npmCli, privateEnvironment } from '../scripts/core-package-check.js';
import { startInstalledServer, startInstalledRemote, startInstalledTrustedServer,
  startInstalledTrustedRemote, localToken, serverToken } from '../scripts/fixtures/installed-server.js';
import { assertTransportClosed, createScenario, exchange, exerciseDelayedJson, exerciseFlow,
  toolRequest, transportResources } from '../scripts/fixtures/transport-scenario.js';

for (const routing of ['passthrough', 'collapsed', 'swapped'] as const) {
  test(`delayed JSON validation checks upstream routes: ${routing}`, { timeout: 3000 }, async () => {
    const initialResources = transportResources();
    const state: Awaited<ReturnType<typeof createScenario>>['state'] = {
      ready: true, cancelled: 0, release() {}, requests: [], failures: [],
    };
    const upstream = createServer(async (incoming, response) => {
      const chunks: Buffer[] = [];
      for await (const chunk of incoming) chunks.push(Buffer.from(chunk));
      let path = incoming.url!;
      if (routing === 'collapsed') path = '/v1/responses';
      if (routing === 'swapped') {
        path = path === '/v1/responses' ? '/v1/chat/completions' : '/v1/responses';
      }
      state.requests.push({ path,
        body: Buffer.concat(chunks), authorization: incoming.headers.authorization });
      response.end('{"done":true}');
    });
    const close = async () => {
      upstream.closeAllConnections();
      await new Promise<void>(resolve => upstream.close(() => resolve()));
    };
    try {
      upstream.listen(0, '127.0.0.1');
      await once(upstream, 'listening');
      const url = `http://127.0.0.1:${(upstream.address() as AddressInfo).port}`;
      const targets = [{ url, token: 'synthetic' }, { url, token: 'synthetic' }];
      const pending = exerciseDelayedJson(targets, { state, upstream, url, close });
      if (routing !== 'passthrough') await assert.rejects(pending, { code: 'ERR_ASSERTION' });
      else assert.equal(await pending, 4);
    } finally { await close(); }
    await assertTransportClosed(initialResources);
  });
}

test('installed Remote forwards through the installed server with isolated credentials and no environment proxy',
  { timeout: 90_000 }, async (context) => {
    const initialResources = transportResources();
    const root = mkdtempSync(join(tmpdir(), 'framerize-installed-pair-'));
    const consumer = join(root, 'consumer with spaces # %');
    let scenario: Awaited<ReturnType<typeof createScenario>> | undefined;
    let server: Awaited<ReturnType<typeof startInstalledServer>> | undefined;
    let remote: Awaited<ReturnType<typeof startInstalledRemote>> | undefined;
    let proxyCalls = 0;
    const proxy = createServer((incoming, outgoing) => { proxyCalls += 1; incoming.resume(); outgoing.end('proxy-control'); });
    try {
      mkdirSync(consumer);
      writeFileSync(join(root, 'user.npmrc'), '');
      writeFileSync(join(root, 'global.npmrc'), '');
      writeFileSync(join(consumer, 'package.json'), '{"private":true,"type":"module"}');
      const env = privateEnvironment(root);
      const repository = process.env.FRAMERIZE_PACKAGE_REPOSITORY ?? fileURLToPath(new URL('../../', import.meta.url));
      const npm = (args: string[], cwd: string) => execFileSync(process.execPath, [npmCli(), ...args],
        { cwd, env, encoding: 'utf8', timeout: 30_000 });
      const archives: string[] = [];
      const identities: object[] = [];
      for (const name of ['framerize-server', 'framerize-remote']) {
        const supplied = process.env[name === 'framerize-server' ? 'SERVER_TGZ' : 'REMOTE_TGZ'];
        if (supplied) {
          const bytes = readFileSync(supplied);
          archives.push(supplied);
          identities.push({ name, sha256: createHash('sha256').update(bytes).digest('hex'), bytes: bytes.length });
          continue;
        }
        assert.equal(Boolean(process.env.SERVER_TGZ || process.env.REMOTE_TGZ), false,
          'Supply both package archives or neither');
        const packed = JSON.parse(npm(['pack', '--offline', '--ignore-scripts', '--json',
          '--pack-destination', root], join(repository, name))) as {
          filename: string; size: number; files: { path: string }[];
        }[];
        assert.equal(packed.length, 1);
        const info = packed[0]!;
        const archive = join(root, info.filename);
        const limit = name === 'framerize-server' ? 2 * 1024 * 1024 : 64 * 1024;
        assert.ok(info.size > 0 && info.size < limit, `${name} archive exceeds its size budget`);
        archives.push(archive);
        identities.push({ name, sha256: createHash('sha256').update(readFileSync(archive)).digest('hex'),
          bytes: info.size, files: info.files.length });
      }
      npm(['install', '--offline', '--ignore-scripts', '--omit=dev', '--package-lock=false',
        '--no-audit', '--no-fund', ...archives], consumer);
      verifyStandaloneTypes(repository, consumer, env);
      context.diagnostic(verifyRemoteNodeApi(consumer, env).trim());
      scenario = await createScenario();
      server = await startInstalledServer(consumer, env, scenario.url);
      proxy.listen(0, '127.0.0.1');
      await once(proxy, 'listening');
      const proxyUrl = `http://127.0.0.1:${(proxy.address() as AddressInfo).port}`;
      const hostile = { ...env, NODE_USE_ENV_PROXY: '1', HTTP_PROXY: proxyUrl, HTTPS_PROXY: proxyUrl, NO_PROXY: '' };
      if (Number(process.versions.node.split('.')[0]) >= 24) {
        const probe = `require('node:http').get(${JSON.stringify(server.url + '/health')}, response => {
          let body = ''; response.on('data', chunk => body += chunk); response.on('end', () => {
            if (body !== 'proxy-control') process.exitCode = 1;
          }); }).on('error', () => process.exitCode = 1);`;
        await promisify(execFile)(process.execPath, ['-e', probe], { env: hostile, cwd: consumer, timeout: 5000 });
        assert.equal(proxyCalls, 1, 'positive control must reach the environment proxy');
        proxyCalls = 0;
      }
      remote = await startInstalledRemote(consumer, hostile, server.url);
      const result = await exerciseFlow(remote.url, localToken, scenario);
      const delayedJsonRequests = await exerciseDelayedJson([
        { url: server.url, token: serverToken }, { url: remote.url, token: localToken },
      ], scenario);
      scenario.state.ready = false;
      const count = scenario.state.requests.filter((entry) => entry.path !== '/v1/models').length;
      const unavailable = await exchange(remote.url, '/v1/chat/completions', localToken, { body: toolRequest });
      assert.equal(unavailable.status, 503);
      assert.equal(JSON.parse(unavailable.body.toString()).error.code, 'upstream_unavailable');
      assert.equal(scenario.state.requests.filter((entry) => entry.path !== '/v1/models').length, count);
      assert.equal(proxyCalls, 0, 'Remote must not contact the environment proxy');
      await remote.close();
      await assert.rejects(exchange(remote.url, '/health', localToken), { code: 'ECONNREFUSED' });
      const elsewhere = join(consumer, 'different working directory');
      mkdirSync(elsewhere);
      scenario.state.ready = true;
      await verifyInstalledDiagnosis(consumer, hostile);
      const savedProfile = join(consumer, 'remote-home/framerize-remote.json');
      const profileBytes = readFileSync(savedProfile);
      remote = await startInstalledRemote(consumer, hostile, server.url, { saved: true, cwd: elsewhere,
        serveArgs: ['--host', '127.0.0.1', '--port', '0'] });
      assert.deepEqual(readFileSync(savedProfile), profileBytes);
      assert.doesNotMatch(remote.output(), /non-loopback-http/);
      assert.equal((await exchange(remote.url, '/ready', localToken)).status, 200);
      assert.equal(proxyCalls, 0, 'Saved profiles must retain proxy isolation');
      await remote.close();
      await server.close();
      await assert.rejects(exchange(remote.url, '/health', localToken), { code: 'ECONNREFUSED' });
      context.diagnostic(JSON.stringify({ node: process.version, identities, ...result,
        delayedJsonRequests, proxyCalls, savedProfileRestart: true,
        temporaryListenOverride: true, closedCli: true }));
    } finally {
      const cleanup = await Promise.allSettled([remote?.close(), server?.close(), scenario?.close()]);
      proxy.closeAllConnections();
      await new Promise<void>((resolve) => proxy.close(() => resolve()));
      rmSync(root, { recursive: true, force: true });
      for (const result of cleanup) if (result.status === 'rejected') throw result.reason;
    }
    await assertTransportClosed(initialResources);
  });

test('installed frm and Server forward trusted traffic without token or profile files',
  { timeout: 90_000 }, async context => {
    const baseline = transportResources();
    const root = mkdtempSync(join(tmpdir(), 'framerize-installed-trusted-'));
    const consumer = join(root, 'consumer with spaces');
    const runtime = join(root, 'runtime');
    let scenario: Awaited<ReturnType<typeof createScenario>> | undefined;
    let server: Awaited<ReturnType<typeof startInstalledTrustedServer>> | undefined;
    let remote: Awaited<ReturnType<typeof startInstalledTrustedRemote>> | undefined;
    try {
      mkdirSync(consumer); mkdirSync(runtime);
      writeFileSync(join(root, 'user.npmrc'), ''); writeFileSync(join(root, 'global.npmrc'), '');
      writeFileSync(join(consumer, 'package.json'), '{"private":true,"type":"module"}');
      const repository = process.env.FRAMERIZE_PACKAGE_REPOSITORY ?? fileURLToPath(new URL('../../', import.meta.url));
      const npm = (args: string[], cwd: string) => execFileSync(process.execPath, [npmCli(), ...args],
        { cwd, env: privateEnvironment(root), encoding: 'utf8', timeout: 30_000 });
      const archives: string[] = [];
      for (const name of ['framerize-server', 'framerize-remote']) {
        const supplied = process.env[name === 'framerize-server' ? 'SERVER_TGZ' : 'REMOTE_TGZ'];
        if (supplied) { archives.push(supplied); continue; }
        assert.equal(Boolean(process.env.SERVER_TGZ || process.env.REMOTE_TGZ), false);
        const packed = JSON.parse(npm(['pack', '--offline', '--ignore-scripts', '--json',
          '--pack-destination', root], join(repository, name))) as { filename: string }[];
        assert.equal(packed.length, 1);
        archives.push(join(root, packed[0]!.filename));
      }
      npm(['install', '--offline', '--ignore-scripts', '--omit=dev', '--package-lock=false',
        '--no-audit', '--no-fund', ...archives], consumer);
      const env = privateEnvironment(runtime);
      scenario = await createScenario(null);
      server = await startInstalledTrustedServer(consumer, env, scenario.url);
      remote = await startInstalledTrustedRemote(consumer, env, server.url);
      const result = await exerciseFlow(remote.url, null, scenario);
      assert.equal((await exchange(remote.url, '/ready', 'synthetic-client-only')).status, 200);
      for (const path of ['/v1/responses', '/v1/chat/completions']) {
        const reply = await exchange(remote.url, path, null,
          { body: { fixture: 'completed-json', stream: false, route: path } });
        assert.equal(reply.status, 200);
        assert.equal(reply.body.toString(), '{"done":true}');
      }
      assert.ok(scenario.state.requests.every(entry => entry.authorization === undefined));
      assert.deepEqual(scenario.state.failures, []);
      assert.ok(readdirSync(runtime, { recursive: true, withFileTypes: true }).every(entry => entry.isDirectory()));
      await remote.close(); await server.close();
      await assert.rejects(exchange(remote.url, '/health', null), { code: 'ECONNREFUSED' });
      await assert.rejects(exchange(server.url, '/health', null), { code: 'ECONNREFUSED' });
      assert.ok(readdirSync(runtime, { recursive: true, withFileTypes: true }).every(entry => entry.isDirectory()));
      const identities = archives.map(archive => ({
        sha256: createHash('sha256').update(readFileSync(archive)).digest('hex'),
        bytes: readFileSync(archive).length,
      }));
      context.diagnostic(JSON.stringify({ node: process.version, ...result, completedJsonRequests: 2,
        identities, savedFiles: 0, authentication: 'none', closedCli: true }));
    } finally {
      const cleanup = await Promise.allSettled([remote?.close(), server?.close(), scenario?.close()]);
      rmSync(root, { recursive: true, force: true });
      for (const result of cleanup) if (result.status === 'rejected') throw result.reason;
    }
    await assertTransportClosed(baseline);
  });

async function verifyInstalledDiagnosis(consumer: string, env: NodeJS.ProcessEnv) {
  const hook = join(consumer, 'synthetic-tailscale.mjs');
  const marker = join(consumer, 'tailscale-invoked');
  const script = `process.stdout.write(JSON.stringify({ BackendState: 'Running',
    Self: { TailscaleIPs: ['100.64.0.1'], DNSName: 'synthetic-private-identity' } }))`;
  writeFileSync(hook, `import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
import { writeFileSync } from 'node:fs';
import assert from 'node:assert/strict';
const original = childProcess.spawn;
childProcess.spawn = (command, args, options) => {
  assert.equal(command, process.platform === 'win32' ? 'tailscale.exe' : 'tailscale');
  assert.deepEqual(args, ['status', '--json']);
  assert.equal(options.shell, false);
  assert.ok(Object.keys(options.env).every(name => /^(PATH|SYSTEMROOT|WINDIR|LANG|LC_ALL)$/i.test(name)));
  writeFileSync(${JSON.stringify(marker)}, 'called');
  return original(process.execPath, ['-e', ${JSON.stringify(script)}], options);
};
syncBuiltinESMExports();`);
  const packageRoot = join(consumer, 'node_modules/framerize-remote');
  const manifest = JSON.parse(readFileSync(join(packageRoot, 'package.json'), 'utf8'));
  assert.equal(typeof manifest.bin?.frm, 'string');
  const launcher = join(packageRoot, manifest.bin.frm);
  const checked = await promisify(execFile)(process.execPath,
    ['--import', pathToFileURL(hook).href, launcher, 'check', '--tailscale', '--json'], {
      cwd: consumer, timeout: 5000,
      env: { ...env, FRAMERIZE_REMOTE_HOME: join(consumer, 'remote-home'),
        FIXTURE_SERVER_TOKEN: 'synthetic-server-credential', FIXTURE_LOCAL_TOKEN: localToken },
    });
  const report = JSON.parse(checked.stdout);
  assert.equal(report.ready, true);
  assert.equal(report.tailscale.state, 'running');
  assert.equal(report.tailscale.scope, 'local_only');
  assert.equal(readFileSync(marker, 'utf8'), 'called');
  assert.doesNotMatch(checked.stdout + checked.stderr, /synthetic-|100\.64\.0\.1/);
}

function verifyRemoteNodeApi(consumer: string, env: NodeJS.ProcessEnv): string {
  return execFileSync(process.execPath, ['--input-type=module', '-e', String.raw`
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { once } from 'node:events';
import { setImmediate } from 'node:timers/promises';
const signalCounts = ['SIGINT', 'SIGTERM'].map(name => process.listenerCount(name));
const { createRemoteBackend, parseRemoteEndpoint } = await import('framerize-remote');
const initialResources = process.getActiveResourcesInfo().filter(name => /TCP|Timeout/.test(name)).sort();
const requests = [], failures = [];
const completed = Buffer.from('{"output":[{"content":[{"text":"synthetic"}]}]}');
const streamChunk = Buffer.from('data: {"delta":"synthetic"}\n\n');
let ready = true, streamClosed;
const server = createServer((request, response) => {
  void (async () => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    requests.push({ path: request.url, headers: request.headers, body: Buffer.concat(chunks) });
    if (request.url === '/ready') {
      response.setHeader('content-type', 'application/json');
      response.end(JSON.stringify({ apiVersion: 1, status: ready ? 'ready' : 'unavailable',
        ...(!ready ? { reason: 'upstream_unavailable' } : {}),
        capabilities: { responses: true, chatCompletions: true, streaming: true } }));
    } else if (request.url === '/openai/v1/responses') {
      response.setHeader('content-type', 'application/json'); response.end(completed);
    } else {
      streamClosed = once(response, 'close');
      response.setHeader('content-type', 'text/event-stream'); response.write(streamChunk);
    }
  })().catch(error => { failures.push(error); response.destroy(); });
});
server.listen(0, '127.0.0.1'); await once(server, 'listening');
const backend = createRemoteBackend({
  endpoint: parseRemoteEndpoint('http://127.0.0.1:' + server.address().port, false),
  serverToken: 'synthetic-node-server', headersTimeoutMs: 2000, idleTimeoutMs: 2000,
  readinessBodyMaxBytes: 65536, readinessWholeMs: 3000, inferenceTimeoutMs: 5000,
});
const requestBody = Buffer.from('{"model":"synthetic","temperature":0.25}');
const forward = (path, signal = AbortSignal.timeout(5000)) => backend.forward({
  path, signal, headers: { 'content-type': 'application/json', accept: '*/*',
    authorization: 'Bearer synthetic-node-client', 'x-client-private': 'do-not-forward' },
  body: (async function* () { yield requestBody.subarray(0, 8); yield requestBody.subarray(8); })(),
});
const collect = async body => {
  const chunks = []; for await (const chunk of body) chunks.push(chunk);
  return Buffer.concat(chunks);
};
const bounded = async promise => {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Node API cleanup did not settle')), 2500);
    })]);
  } finally { clearTimeout(timer); }
};
try {
  assert.deepEqual(await backend.diagnose({ signal: AbortSignal.timeout(3000) }), { readiness: true });
  ready = false;
  assert.equal((await backend.diagnose({ signal: AbortSignal.timeout(3000) })).code, 'upstream_unavailable');
  const unavailable = await forward('/v1/responses');
  assert.equal(unavailable.status, 503);
  assert.equal(JSON.parse((await collect(unavailable.body)).toString()).error.code, 'upstream_unavailable');
  assert.ok(requests.every(request => request.path === '/ready'), 'unready backend must not forward');
  ready = true;
  const response = await forward('/openai/v1/responses');
  assert.equal(response.status, 200);
  assert.deepEqual(await collect(response.body), completed);
  for (const action of ['return', 'abort', 'close']) {
    const controller = new AbortController();
    streamClosed = undefined;
    const streaming = await forward('/v1/responses', controller.signal);
    assert.equal(streaming.status, 200);
    assert.ok(streamClosed, 'fixture must have observed this stream');
    const closed = streamClosed;
    const iterator = streaming.body[Symbol.asyncIterator]();
    const first = await bounded(iterator.next());
    assert.equal(first.done, false); assert.deepEqual(first.value, streamChunk);
    if (action === 'return') assert.equal((await bounded(iterator.return())).done, true);
    else {
      if (action === 'abort') controller.abort(); else backend.close();
      await bounded(assert.rejects(iterator.next()));
    }
    await bounded(closed);
  }
  assert.equal((await backend.diagnose({ signal: AbortSignal.timeout(3000) })).code, 'configuration_unavailable');
  assert.deepEqual(requests.filter(request => request.path !== '/ready').map(request => request.path),
    ['/openai/v1/responses', '/v1/responses', '/v1/responses', '/v1/responses']);
  for (const request of requests) {
    assert.equal(request.headers.authorization, 'Bearer synthetic-node-server');
    assert.equal(request.headers['x-client-private'], undefined);
    if (request.path !== '/ready') assert.deepEqual(request.body, requestBody);
  }
} finally {
  backend.close(); backend.close();
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
}
await setImmediate(); await setImmediate();
assert.equal(server.listening, false);
assert.deepEqual(failures, []);
assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => process.listenerCount(name)), signalCounts);
assert.deepEqual(process.getActiveResourcesInfo().filter(name => /TCP|Timeout/.test(name)).sort(), initialResources);
console.log(JSON.stringify({ node: process.version, completed: true, readiness: true,
  streamCleanup: ['return', 'abort', 'close'], retainedResources: [] }));
`], { cwd: consumer, env, encoding: 'utf8', stdio: 'pipe', timeout: 15_000 });
}

function verifyStandaloneTypes(repository: string, consumer: string, env: NodeJS.ProcessEnv) {
  const cli = (args: string[]) => execFileSync(process.execPath,
    [npmCli(), 'exec', '--offline', '--no', '--', 'frm', ...args],
    { cwd: consumer, env, encoding: 'utf8', timeout: 10000 });
  assert.equal(cli(['version']).trim(), '0.2.0');
  assert.match(cli(['setup', '--help']), /--server-token-env/);
  const serveHelp = cli(['serve', '--help']);
  assert.match(serveHelp, /FRM_REMOTE/);
  assert.match(serveHelp, /--port/);
  assert.equal(existsSync(join(consumer, 'node_modules/@framerize/core')), false);
  assert.equal(existsSync(join(consumer, 'node_modules/framerize-server/src')), false);
  execFileSync(process.execPath, ['--input-type=module', '-e', `
import assert from 'node:assert/strict';
import { createFramerAuth } from 'framerize-server/framer-auth';
import { createFramerBackend } from 'framerize-server/framer-backend';
import { createAccountBackend } from 'framerize-server/account-backend';
import { createServer as createAuthServer } from 'node:http';
import { once } from 'node:events';
import { writeFileSync } from 'node:fs';
const auth = createFramerAuth({ sessionCookie: 'synthetic-only' });
const framer = createFramerBackend({ projectId: 'synthetic', auth,
  castle: { async mint() { return { token: 'synthetic' }; }, async close() {} } });
await framer.close();
await assert.rejects(auth.accessToken(), { code: 'auth_closed' });
await assert.rejects(framer.ready({ signal: new AbortController().signal }), { code: 'provider_closed' });
const fixture = createAuthServer((request, response) => {
  request.resume();
  assert.equal(request.url, '/auth/ai/access-token');
  assert.equal(request.headers.cookie, 'session=synthetic');
  response.setHeader('content-type', 'application/json');
  response.end(JSON.stringify({ accessToken: 'synthetic', expiresAt: new Date(Date.now() + 600000).toISOString() }));
});
fixture.listen(0, '127.0.0.1'); await once(fixture, 'listening');
let selected;
try {
  writeFileSync('fixture-accounts.json', JSON.stringify([{ id: 'fixture', projectId: 'project', session: { env: 'SESSION' } }]));
  selected = await createAccountBackend({ accountsFile: 'fixture-accounts.json', env: { SESSION: 'session=synthetic' },
    origin: 'http://127.0.0.1:' + fixture.address().port,
    createMinter: () => ({ async mint() { assert.fail('readiness must not mint'); }, async close() {} }) });
  assert.equal(await selected.ready({ signal: new AbortController().signal }), true);
} finally {
  await selected?.close(); fixture.closeAllConnections(); await new Promise(resolve => fixture.close(resolve));
}

`], { cwd: consumer, env, stdio: 'pipe', timeout: 5000 });
  const packageRoot = join(consumer, 'node_modules/framerize-remote');
  const manifest = JSON.parse(readFileSync(join(packageRoot, 'package.json'), 'utf8'));
  assert.equal(manifest.name, 'framerize-remote');
  assert.equal(typeof manifest.bin?.frm, 'string');
  assert.ok(existsSync(join(packageRoot, manifest.bin.frm)));
  assert.match(cli(['--help']), /^frm 0\.2\.0\n/);
  for (const key of ['dependencies', 'optionalDependencies', 'peerDependencies']) {
    assert.deepEqual(Object.keys(manifest[key] ?? {}), []);
  }
  assert.equal(readFileSync(join(packageRoot, 'T3-LICENSE'), 'utf8'),
    readFileSync(join(repository, 'framerize-remote/T3-LICENSE'), 'utf8'));
  writeFileSync(join(consumer, 'consumer.ts'), `
import { createRemoteBackend, parseRemoteEndpoint, type RemoteConfig,
  type RemoteBackendConfig, type RemoteEndpoint } from 'framerize-remote';
import { createProvider, type ProviderOptions } from 'framerize-server/provider';
import { createFramerAuth, type FramerAuth } from 'framerize-server/framer-auth';
import { createFramerBackend, type FramerMinter } from 'framerize-server/framer-backend';
import { createAccountBackend, type AccountBackendOptions } from 'framerize-server/account-backend';
const auth: FramerAuth = createFramerAuth({ sessionCookie: 'synthetic-only' });
const castle: FramerMinter = { async mint() { return { token: 'synthetic' }; }, async close() {} };
const framer = createFramerBackend({ projectId: 'synthetic', auth, castle });
const accountOptions: AccountBackendOptions = { accountsFile: 'accounts.json', createMinter: () => castle };
const accountBackend: Promise<ServerProvider> = createAccountBackend(accountOptions);
const framerClosed: Promise<void> | void = framer.close();
await auth.close();
import { createServer, type ServerProvider } from 'framerize-server';
import { createLogger } from 'framerize-server/logger';
const logger = createLogger({ level: 'silent' });
const syncProvider: ServerProvider = { ready: () => true,
  forward: () => ({ status: 200, headers: {}, body: [Buffer.from('{}')] }) };
const server = await createServer({}, { provider: syncProvider, logger });
const stopped: Promise<void> = server.close();
const remoteConfig: Pick<RemoteConfig, 'remote'> = { remote: 'http://127.0.0.1' };
const endpoint: RemoteEndpoint = parseRemoteEndpoint(remoteConfig.remote, false);
const backendConfig: RemoteBackendConfig = { endpoint,
  serverToken: 'synthetic', headersTimeoutMs: 100, idleTimeoutMs: 100,
  readinessBodyMaxBytes: 65536, readinessWholeMs: 200 };
const backend = createRemoteBackend(backendConfig);
backend.close();
const options: ProviderOptions = { upstream: 'http://127.0.0.1',
  credential: ({ signal }) => { signal.throwIfAborted(); return 'synthetic'; } };
const provider: Pick<typeof backend, 'ready' | 'forward' | 'close'> = createProvider(options);
const standalone = createProvider(options);
standalone.ready();
standalone.forward({ path: '/v1/responses', body: (async function* () { yield Buffer.from('{}'); })() });
provider.close();
`);
  execFileSync(process.execPath, [join(repository, 'node_modules/typescript/bin/tsc'), 'consumer.ts',
    '--noEmit', '--strict', '--module', 'NodeNext', '--moduleResolution', 'NodeNext',
    '--target', 'ES2022', '--typeRoots', join(repository, 'node_modules/@types')],
  { cwd: consumer, env, stdio: 'pipe', timeout: 30_000 });
}

test('Git checkout preserves exact T3 license bytes with automatic CRLF enabled', () => {
  const root = mkdtempSync(join(tmpdir(), 'framerize-license-checkout-'));
  const repository = process.env.FRAMERIZE_PACKAGE_REPOSITORY ?? fileURLToPath(new URL('../../', import.meta.url));
  const env = { ...privateEnvironment(root), GIT_CONFIG_NOSYSTEM: '1' };
  const git = (args: string[]) => execFileSync('git', args, { cwd: root, env, timeout: 5000, stdio: 'pipe' });
  try {
    const relative = 'framerize-remote/T3-LICENSE';
    const original = execFileSync('git', ['show', `HEAD:${relative}`], { cwd: repository, env, timeout: 5000 });
    const crlf = Buffer.from(original.toString().replace(/\n/g, '\r\n'));
    assert.equal(original.includes(13), false);
    git(['init', '--quiet']);
    mkdirSync(join(root, 'framerize-remote'));
    writeFileSync(join(root, relative), original);
    git(['-c', 'core.autocrlf=false', 'add', relative]);
    for (const configured of [false, true]) {
      writeFileSync(join(root, '.gitattributes'), configured ? readFileSync(join(repository, '.gitattributes')) : '');
      rmSync(join(root, relative));
      git(['-c', 'core.autocrlf=true', 'checkout-index', '--force', '--', relative]);
      assert.deepEqual(readFileSync(join(root, relative)), configured ? original : crlf);
    }
  } finally { rmSync(root, { recursive: true, force: true }); }
});
