import assert from 'node:assert/strict';
import test from 'node:test';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { npmCli, privateEnvironment } from '../scripts/core-package-check.js';
import { readPackageArchive } from '../scripts/package-archive.js';
import { createCustomScenario, exerciseCustomFlow } from '../scripts/fixtures/custom-route-scenario.js';
import { startInstalledAccountServer, startInstalledRemote, localToken, serverToken } from '../scripts/fixtures/installed-server.js';
import { assertTransportClosed, exchange, transportResources } from '../scripts/fixtures/transport-scenario.js';

function installPackages(repository: string, root: string, consumer: string, env: NodeJS.ProcessEnv) {
  const npm = (args: string[], cwd: string) => execFileSync(process.execPath, [npmCli(), ...args],
    { cwd, env, encoding: 'utf8', timeout: 30_000 });
  const packages = ['framerize-server', 'framerize-remote'].map(name => {
    let archive = process.env[name === 'framerize-server' ? 'SERVER_TGZ' : 'REMOTE_TGZ'];
    if (!archive) {
      assert.equal(Boolean(process.env.SERVER_TGZ || process.env.REMOTE_TGZ), false);
      const packed = JSON.parse(npm(['pack', '--offline', '--ignore-scripts', '--json',
        '--pack-destination', root], join(repository, name))) as { filename: string }[];
      assert.equal(packed.length, 1);
      archive = join(root, packed[0]!.filename);
    }
    const bytes = readFileSync(archive);
    assert.ok(bytes.length < (name === 'framerize-server' ? 2 * 1024 * 1024 : 64 * 1024));
    const members = readPackageArchive(archive);
    return { name, archive, members, bytes: bytes.length,
      sha256: createHash('sha256').update(bytes).digest('hex') };
  });

  npm(['install', '--offline', '--ignore-scripts', '--omit=dev', '--package-lock=false',
    '--no-audit', '--no-fund', ...packages.map(value => value.archive)], consumer);
  for (const entry of packages) {
    for (const [path, bytes] of entry.members) {
      const relative = path.slice('package/'.length);
      assert.deepEqual(bytes, readFileSync(join(repository, entry.name, relative)));
      assert.deepEqual(bytes, readFileSync(join(consumer, 'node_modules', entry.name, relative)));
    }
  }
  assert.equal(existsSync(join(consumer, 'node_modules/typescript')), false);
  assert.equal(existsSync(join(consumer, 'node_modules/@framerize/core')), false);
  return packages.map(({ name, bytes, sha256, members }) => ({ name, bytes, sha256, files: members.size }));
}

test('installed custom routes use the account server and Remote without a workspace',
  { timeout: 240_000 }, async context => {
    const resources = transportResources();
    const root = mkdtempSync(join(tmpdir(), 'framerize-custom-installed-'));
    const consumer = join(root, 'consumer with spaces # %');
    const env = privateEnvironment(root);
    let scenario: Awaited<ReturnType<typeof createCustomScenario>> | undefined;
    let server: Awaited<ReturnType<typeof startInstalledAccountServer>> | undefined;
    let remote: Awaited<ReturnType<typeof startInstalledRemote>> | undefined;
    try {
      mkdirSync(consumer);
      writeFileSync(join(root, 'user.npmrc'), '');
      writeFileSync(join(root, 'global.npmrc'), '');
      writeFileSync(join(consumer, 'package.json'), '{"private":true,"type":"module"}');
      const repository = fileURLToPath(new URL('../../', import.meta.url));
      const packages = installPackages(repository, root, consumer, env);
      scenario = await createCustomScenario();
      server = await startInstalledAccountServer(consumer, env, scenario.url);
      await exerciseCustomFlow(server.url, serverToken, scenario);
      remote = await startInstalledRemote(consumer, env, server.url);
      await exerciseCustomFlow(remote.url, localToken, scenario);
      assert.deepEqual(scenario.state.failures, []);
      assert.equal(scenario.state.auth, 1);
      assert.equal(scenario.state.inference, 24);
      assert.equal(scenario.state.cancelled, 4);
      await remote.close();
      await server.close();
      assert.doesNotMatch(server.output() + remote.output(), /synthetic-access|session=synthetic-account/);
      await assert.rejects(exchange(server.url, '/health', serverToken), { code: 'ECONNREFUSED' });
      await assert.rejects(exchange(remote.url, '/health', localToken), { code: 'ECONNREFUSED' });
      context.diagnostic(JSON.stringify({ node: process.version, packages,
        completedToolRoundTrips: 4, streamedToolRoundTrips: 4, invalidReplies: 4, cancelled: scenario.state.cancelled,
        providerPosts: scenario.state.inference, builtInMinter: true }));
    } finally {
      const cleanup = await Promise.allSettled([remote?.close(), server?.close(), scenario?.close()]);
      rmSync(root, { recursive: true, force: true });
      for (const result of cleanup) if (result.status === 'rejected') throw result.reason;
    }
    await assertTransportClosed(resources);
  });

test('custom-route checks are selected by the core CI matrix', () => {
  const workflow = readFileSync(new URL('../../.github/workflows/core-ci.yml', import.meta.url), 'utf8');
  const coordinator = readFileSync(new URL('../../.github/workflows/ci.yml', import.meta.url), 'utf8');
  const manifest = JSON.parse(readFileSync(new URL('../../package.json', import.meta.url), 'utf8'));
  const config = JSON.parse(readFileSync(new URL('../../tsconfig.tools.json', import.meta.url), 'utf8'));
  assert.match(workflow, /\n  workflow_call:/);
  assert.match(coordinator, /uses: \.\/\.github\/workflows\/core-ci\.yml/);
  assert.ok(config.include.includes('test/installed-custom-routes.test.ts'));
  assert.match(manifest.scripts.test, /npm run test:custom-routes && npm run test:custom-installed/);
  assert.match(manifest.scripts['test:custom-installed'], /\.tools\/test\/installed-custom-routes\.test\.js/);
  assert.match(manifest.scripts['test:custom-routes'], /test\/custom-streaming\.test\.mjs/);
});
