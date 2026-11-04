import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const launcher = fileURLToPath(new URL('../bin/framerize-server.js', import.meta.url));
const invalid = {
  malformed: '{"private-content-canary":',
  shape: '{}',
  metadata: '[{"id":"demo","private-content-canary":true}]',
  'device-id': '[{"id":"demo","castleDeviceId":"private-content-canary invalid"}]',
  reference: '[{"id":"demo","session":{"private-content-canary":true}}]',
};

for (const backend of ['http', 'framer']) for (const failure of ['missing', ...Object.keys(invalid)]) {
  test(`${backend} launcher explains and recovers from ${failure} account metadata`, context => {
    const root = mkdtempSync(join(tmpdir(), 'server-account-diagnostic-'));
    context.after(() => rmSync(root, { recursive: true, force: true }));
    const accountsFile = join(root, 'private-path-canary');
    if (failure !== 'missing') writeFileSync(accountsFile, invalid[failure]);
    const backendOptions = backend === 'framer'
      ? ['--account-id', 'demo', '--framer-origin', 'http://127.0.0.1:1'] : [];
    const invoke = command => spawnSync(process.execPath,
      [launcher, command, '--accounts-file', accountsFile, '--host', '127.0.0.1',
        '--port', '0', '--server-token-env', 'DIAGNOSTIC_TOKEN', '--backend', backend, ...backendOptions], {
        cwd: root, encoding: 'utf8', timeout: 5000,
        env: { HOME: root, USERPROFILE: root, TMPDIR: root,
          DIAGNOSTIC_TOKEN: 'synthetic-token-canary', DIAGNOSTIC_SESSION: 'session=synthetic-session-canary',
          NODE_DISABLE_COMPILE_CACHE: '1' },
      });
    for (const command of ['check', 'serve']) {
      const result = invoke(command);
      assert.equal(result.error, undefined);
      assert.equal(result.signal, null);
      assert.equal(result.status, 1);
      assert.equal(result.stdout, '');
      const diagnostic = JSON.parse(result.stderr);
      assert.equal(diagnostic.event, 'server.failed');
      assert.equal(diagnostic.message, 'Account metadata file is unavailable or invalid.');
      assert.match(diagnostic.next, /--accounts-file/);
      assert.match(diagnostic.next, /read permissions/);
      assert.match(diagnostic.next, /JSON/);
      assert.doesNotMatch(result.stderr,
        /private-path-canary|private-content-canary|synthetic-token-canary|synthetic-session-canary/);
    }
    writeFileSync(accountsFile, JSON.stringify(backend === 'framer'
      ? [{ id: 'demo', projectId: 'synthetic-project', session: { env: 'DIAGNOSTIC_SESSION' } }] : []));
    const recovered = invoke('check');
    assert.equal(recovered.error, undefined);
    assert.equal(recovered.signal, null);
    assert.equal(recovered.status, 0, recovered.stderr);
    assert.equal(JSON.parse(recovered.stderr).event, 'config.valid');
    assert.doesNotMatch(recovered.stdout + recovered.stderr,
      /private-path-canary|private-content-canary|synthetic-token-canary|synthetic-session-canary/);
  });
}
