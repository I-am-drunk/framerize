import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, copyFileSync, lstatSync, mkdirSync, mkdtempSync, readFileSync,
  readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { npmCli, privateEnvironment } from './core-package-check.js';
import { localToken, startInstalledRemote, startInstalledServer } from './fixtures/installed-server.js';
import { assertTransportClosed, createScenario, exchange, exerciseFlow,
  toolRequest, transportResources } from './fixtures/transport-scenario.js';

export async function checkNativeServer(executable: string, remoteArchive: string) {
  for (const file of [executable, remoteArchive]) assert.ok(lstatSync(file).isFile(), 'Input must be a regular file');
  const initialResources = transportResources();
  const root = mkdtempSync(join(tmpdir(), 'framerize native pair '));
  const consumer = join(root, 'remote');
  const native = join(root, 'server');
  let scenario: Awaited<ReturnType<typeof createScenario>> | undefined;
  let server: Awaited<ReturnType<typeof startInstalledServer>> | undefined;
  let remote: Awaited<ReturnType<typeof startInstalledRemote>> | undefined;
  let result: object;
  try {
    mkdirSync(consumer); mkdirSync(native);
    const binary = join(native, process.platform === 'win32' ? 'framerize-server.exe' : 'framerize-server');
    copyFileSync(executable, binary); chmodSync(binary, 0o700);
    writeFileSync(join(root, 'user.npmrc'), '');
    writeFileSync(join(root, 'global.npmrc'), '');
    writeFileSync(join(consumer, 'package.json'), '{"private":true,"type":"module"}');
    const env = privateEnvironment(root);
    execFileSync(process.execPath, [npmCli(), 'install', '--offline', '--ignore-scripts', '--omit=dev',
      '--package-lock=false', '--no-audit', '--no-fund', resolve(remoteArchive)],
    { cwd: consumer, env, stdio: 'pipe', timeout: 30_000 });
    const manifest = JSON.parse(readFileSync(join(consumer, 'node_modules/framerize-remote/package.json'), 'utf8'));
    assert.equal(manifest.name, 'framerize-remote');
    for (const field of ['dependencies', 'optionalDependencies', 'peerDependencies']) {
      assert.deepEqual(Object.keys(manifest[field] ?? {}), [], 'Remote must have no runtime dependencies');
    }
    const isolated = { ...env, PATH: '' };
    assert.match(execFileSync(binary, ['--help'], { cwd: native, env: isolated, encoding: 'utf8', timeout: 5000 }),
      /Usage: framerize-server/);
    scenario = await createScenario();
    server = await startInstalledServer(native, isolated, scenario.url, binary);
    remote = await startInstalledRemote(consumer, env, server.url);
    const flow = await exerciseFlow(remote.url, localToken, scenario);
    scenario.state.ready = false;
    const before = scenario.state.requests.filter(entry => entry.path !== '/v1/models').length;
    const refused = await exchange(remote.url, '/v1/chat/completions', localToken, { body: toolRequest });
    assert.equal(refused.status, 503);
    assert.equal(JSON.parse(refused.body.toString()).error.code, 'upstream_unavailable');
    assert.equal(scenario.state.requests.filter(entry => entry.path !== '/v1/models').length, before);
    assert.deepEqual(readdirSync(native).sort(), ['accounts.json', process.platform === 'win32'
      ? 'framerize-server.exe' : 'framerize-server'].sort());
    await remote.close();
    await server.close();
    await assert.rejects(exchange(server.url, '/health', localToken), { code: 'ECONNREFUSED' });
    const digest = (file: string) => createHash('sha256').update(readFileSync(file)).digest('hex');
    result = { node: process.version, platform: process.platform, ...flow,
      executableSha256: digest(binary), remoteArchiveSha256: digest(remoteArchive),
      nativePathEmpty: true, closedCli: true };
  } finally {
    const cleanup = await Promise.allSettled([remote?.close(), server?.close(), scenario?.close()]);
    rmSync(root, { recursive: true, force: true });
    for (const entry of cleanup) if (entry.status === 'rejected') throw entry.reason;
  }
  await assertTransportClosed(initialResources);
  return result;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  assert.equal(process.argv.length, 4, 'Usage: check-native-server EXECUTABLE REMOTE_TARBALL');
  console.log(JSON.stringify(await checkNativeServer(resolve(process.argv[2]!), resolve(process.argv[3]!))));
}
