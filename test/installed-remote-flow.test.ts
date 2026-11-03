import assert from 'node:assert/strict';
import test from 'node:test';
import { execFile, execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { once } from 'node:events';
import type { AddressInfo } from 'node:net';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setImmediate } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { npmCli, privateEnvironment } from '../scripts/core-package-check.js';
import { startInstalledServer, startInstalledRemote, localToken } from '../scripts/fixtures/installed-server.js';
import { createScenario, exchange, exerciseFlow, toolRequest } from '../scripts/fixtures/transport-scenario.js';

test('installed Remote forwards through the installed server with isolated credentials and no environment proxy',
  { timeout: 90_000 }, async (context) => {
    const initialResources = process.getActiveResourcesInfo().filter((name) => /TCP|Timeout/.test(name)).sort();
    const root = mkdtempSync(join(tmpdir(), 'framerize-installed-pair-'));
    const consumer = join(root, 'consumer with spaces');
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
        assert.ok(info.size < 65536);
        archives.push(archive);
        identities.push({ name, sha256: createHash('sha256').update(readFileSync(archive)).digest('hex'),
          bytes: info.size, files: info.files.length });
      }
      npm(['install', '--offline', '--ignore-scripts', '--omit=dev', '--package-lock=false',
        '--no-audit', '--no-fund', ...archives], consumer);
      verifyStandaloneTypes(repository, consumer, env);
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
        proxyCalls, savedProfileRestart: true, temporaryListenOverride: true, closedCli: true }));
    } finally {
      const cleanup = await Promise.allSettled([remote?.close(), server?.close(), scenario?.close()]);
      proxy.closeAllConnections();
      await new Promise<void>((resolve) => proxy.close(() => resolve()));
      rmSync(root, { recursive: true, force: true });
      for (const result of cleanup) if (result.status === 'rejected') throw result.reason;
    }
    await setImmediate();
    await setImmediate();
    assert.deepEqual(process.getActiveResourcesInfo().filter((name) => /TCP|Timeout/.test(name)).sort(), initialResources);
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
  const launcher = join(consumer, 'node_modules/framerize-remote/bin/framerize-remote.js');
  const checked = await promisify(execFile)(process.execPath,
    ['--import', hook, launcher, 'check', '--tailscale', '--json'], {
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

function verifyStandaloneTypes(repository: string, consumer: string, env: NodeJS.ProcessEnv) {
  const cli = (args: string[]) => execFileSync(process.execPath,
    [npmCli(), 'exec', '--offline', '--no', '--', 'framerize-remote', ...args],
    { cwd: consumer, env, encoding: 'utf8', timeout: 10000 });
  assert.equal(cli(['version']).trim(), '0.2.0');
  assert.match(cli(['setup', '--help']), /--server-token-env/);
  assert.match(cli(['serve', '--help']), /Overrides never change the saved profile/);
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
  assert.deepEqual(manifest.bin, { 'framerize-remote': './bin/framerize-remote.js' });
  assert.match(cli(['--help']), /^framerize-remote 0\.2\.0\n/);
  for (const key of ['dependencies', 'optionalDependencies', 'peerDependencies']) {
    assert.deepEqual(Object.keys(manifest[key] ?? {}), []);
  }
  assert.equal(readFileSync(join(packageRoot, 'T3-LICENSE'), 'utf8'),
    readFileSync(join(repository, 'framerize-remote/T3-LICENSE'), 'utf8'));
  writeFileSync(join(consumer, 'consumer.ts'), `
import { createRemoteBackend, parseRemoteEndpoint, type RemoteConfig } from 'framerize-remote';
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
const backend = createRemoteBackend({ endpoint: parseRemoteEndpoint(remoteConfig.remote, false),
  serverToken: 'synthetic', headersTimeoutMs: 100, idleTimeoutMs: 100,
  readinessBodyMaxBytes: 65536, readinessWholeMs: 200 });
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
