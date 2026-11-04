import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, copyFileSync, lstatSync, mkdirSync, mkdtempSync,
  readFileSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { privateEnvironment } from './core-package-check.js';
import { createAccountScenario, exerciseAccountFlow } from './fixtures/account-scenario.js';
import { accountServerFixture, serverToken, startInstalledAccountServer } from './fixtures/installed-server.js';
import { assertTransportClosed, transportResources } from './fixtures/transport-scenario.js';

export async function checkNativeAccount(executable: string) {
  assert.ok(lstatSync(executable).isFile(), 'Native executable must be a regular file');
  const baseline = transportResources();
  const root = mkdtempSync(join(tmpdir(), 'framerize native account '));
  const state = join(root, 'state');
  const binary = join(root, process.platform === 'win32' ? 'server.exe' : 'server');
  let scenario: Awaited<ReturnType<typeof createAccountScenario>> | undefined;
  let server: Awaited<ReturnType<typeof startInstalledAccountServer>> | undefined;
  try {
    mkdirSync(state);
    copyFileSync(executable, binary); chmodSync(binary, 0o700);
    const env = { ...privateEnvironment(state), PATH: '', Path: '' };
    scenario = await createAccountScenario();
    const config = accountServerFixture(root, env, scenario.url);
    const output = execFileSync(binary, ['check', ...config.args], {
      cwd: root, env: config.env, encoding: 'utf8', timeout: 10000, maxBuffer: 65536,
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    assert.doesNotMatch(output, /synthetic-account|synthetic-access/);
    assert.equal(scenario.state.auth, 0, 'Configuration check contacted the provider');
    server = await startInstalledAccountServer(root, env, scenario.url, binary);
    const flow = await exerciseAccountFlow(server.url, serverToken, scenario);
    await server.close();
    assert.doesNotMatch(server.output(), /synthetic-account|synthetic-access/);
    return { ...flow, platform: process.platform, node: process.version,
      executableSha256: createHash('sha256').update(readFileSync(binary)).digest('hex'),
      nativePathEmpty: true, normalExitCleanup: true,
      signalCleanup: process.platform === 'win32' ? 'not-tested' : 'passed' };
  } finally {
    const cleanup = await Promise.allSettled([server?.close(), scenario?.close()]);
    rmSync(root, { recursive: true, force: true, maxRetries: 3, retryDelay: 100 });
    await assertTransportClosed(baseline);
    for (const entry of cleanup) if (entry.status === 'rejected') throw entry.reason;
  }
}
