import assert from 'node:assert/strict';
import test from 'node:test';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { setImmediate } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { npmCli, privateEnvironment } from '../scripts/core-package-check.js';
import { startInstalledServer, serverToken } from '../scripts/fixtures/installed-server.js';
import { createScenario, exchange, exerciseFlow } from '../scripts/fixtures/transport-scenario.js';

test('installed server CLI preserves a tool round trip and cancels the real upstream',
  { timeout: 90_000 }, async (context) => {
    const initialResources = process.getActiveResourcesInfo().filter((name) => /TCP|Timeout/.test(name)).sort();
    const root = mkdtempSync(join(tmpdir(), 'framerize-installed-flow-'));
    const consumer = join(root, 'consumer');
    let scenario: Awaited<ReturnType<typeof createScenario>> | undefined;
    let server: Awaited<ReturnType<typeof startInstalledServer>> | undefined;
    try {
      mkdirSync(consumer);
      writeFileSync(join(root, 'user.npmrc'), '');
      writeFileSync(join(root, 'global.npmrc'), '');
      writeFileSync(join(consumer, 'package.json'), '{"private":true,"type":"module"}');
      const env = privateEnvironment(root);
      const npm = (args: string[], cwd: string) => execFileSync(process.execPath, [npmCli(), ...args],
        { cwd, env, encoding: 'utf8', timeout: 30_000 });
      const source = fileURLToPath(new URL('../../framerize-server', import.meta.url));
      const packed = JSON.parse(npm(['pack', '--offline', '--ignore-scripts', '--json',
        '--pack-destination', root], source)) as { filename: string }[];
      assert.equal(packed.length, 1);
      const archive = join(root, packed[0]!.filename);
      const sha256 = createHash('sha256').update(readFileSync(archive)).digest('hex');
      npm(['install', '--offline', '--ignore-scripts', '--omit=dev', '--package-lock=false',
        '--no-audit', '--no-fund', archive], consumer);
      scenario = await createScenario();
      server = await startInstalledServer(consumer, env, scenario.url);
      const result = await exerciseFlow(server.url, serverToken, scenario);
      await server.close();
      await assert.rejects(exchange(server.url, '/health', serverToken), { code: 'ECONNREFUSED' });
      context.diagnostic(JSON.stringify({ node: process.version, serverSha256: sha256, ...result, closedCli: true }));
    } finally {
      const cleanup = await Promise.allSettled([server?.close(), scenario?.close()]);
      rmSync(root, { recursive: true, force: true });
      for (const result of cleanup) if (result.status === 'rejected') throw result.reason;
    }
    await setImmediate();
    await setImmediate();
    assert.deepEqual(process.getActiveResourcesInfo().filter((name) => /TCP|Timeout/.test(name)).sort(), initialResources);
  });

test('installed transport runs in the six-job core CI matrix', () => {
  const workflow = readFileSync(new URL('../../.github/workflows/core-ci.yml', import.meta.url), 'utf8');
  const manifest = JSON.parse(readFileSync(new URL('../../package.json', import.meta.url), 'utf8'));
  assert.match(manifest.scripts.test, /node --test .*\.tools\/test\/installed-server-flow\.test\.js/);
  assert.match(manifest.scripts['check:native'], /node \.tools\/scripts\/check-native-server\.js/);
  assert.equal(workflow.split("- 'scripts/check-native-server.ts'").length - 1, 2);
  assert.equal(workflow.split("- 'scripts/fixtures/**'").length - 1, 2);
  assert.equal(workflow.split("- 'test/installed-server-flow.test.ts'").length - 1, 2);
  assert.match(workflow, /node: \['22\.23\.4', '24\.13\.0'\]/);
  assert.match(workflow, /os: \[ubuntu-22\.04, macos-14, windows-2022\]/);
});
