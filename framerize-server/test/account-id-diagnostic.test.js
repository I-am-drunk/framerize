import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const packageRoot = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
const { loadConfig } = await import(pathToFileURL(join(packageRoot, 'dist/config.js')));
const { AccountSelectionError, ConfigurationError } = await import(
  pathToFileURL(join(packageRoot, 'dist/errors.js')));
const launcher = join(packageRoot, 'bin/framerize-server.js');
const invalidId = 'invalid account-id-canary';

for (const source of ['flag', 'environment']) {
  test(`invalid ${source} account ID retains configuration error compatibility`, () => {
    const input = { backend: 'framer', accountsFile: 'unread-file-canary' };
    const env = source === 'environment' ? { FRAMERIZE_SERVER_ACCOUNT: invalidId } : {};
    if (source === 'flag') input.accountId = invalidId;
    assert.throws(() => loadConfig(input, env), error => {
      assert.ok(error instanceof AccountSelectionError);
      assert.ok(error instanceof ConfigurationError);
      assert.equal(error.name, 'ConfigurationError');
      assert.equal(error.code, 'configuration_error');
      assert.equal(error.status, 500);
      assert.equal(error.message, `Invalid ${source === 'flag' ? 'accountId' : 'FRAMERIZE_SERVER_ACCOUNT'}`);
      return true;
    });
  });
}

for (const source of ['flag', 'environment']) for (const command of ['check', 'serve']) {
  test(`${command} explains malformed ${source} account IDs and accepts a correction`, context => {
    const root = mkdtempSync(join(tmpdir(), 'account-id-diagnostic-'));
    context.after(() => rmSync(root, { recursive: true, force: true }));
    const file = join(root, 'private-path-canary');
    writeFileSync(file, JSON.stringify([{ id: 'selected', projectId: 'project-canary',
      session: { env: 'DIAGNOSTIC_SESSION' } }]));
    const invoke = (verb, id) => spawnSync(process.execPath,
      [launcher, verb, '--backend', 'framer', '--accounts-file', file,
        '--server-token-env', 'DIAGNOSTIC_TOKEN', '--framer-origin', 'http://127.0.0.1:1',
        '--host', '127.0.0.1', '--port', '0', ...(source === 'flag' ? ['--account-id', id] : [])], {
        cwd: root, encoding: 'utf8', timeout: 5000,
        env: { HOME: root, USERPROFILE: root, TMPDIR: root,
          DIAGNOSTIC_TOKEN: 'synthetic-token-canary', DIAGNOSTIC_SESSION: 'session=synthetic-canary',
          ...(source === 'environment' ? { FRAMERIZE_SERVER_ACCOUNT: id } : {}),
          ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}),
          NODE_DISABLE_COMPILE_CACHE: '1' },
      });
    const result = invoke(command, invalidId);
    assert.equal(result.error, undefined);
    assert.equal(result.signal, null);
    assert.equal(result.status, 1);
    assert.equal(result.stdout, '');
    const detail = JSON.parse(result.stderr);
    assert.equal(detail.event, 'server.failed');
    assert.equal(detail.message, 'Select one configured Framer account.');
    assert.match(detail.next, /--account-id/);
    assert.match(detail.next, /\bFRAMERIZE_SERVER_ACCOUNT\b/);
    assert.match(detail.next, /\bFRAMERIZE_SERVER_ACCOUNTS\b/);
    assert.doesNotMatch(result.stderr, /canary/);
    const recovered = invoke('check', 'selected');
    assert.equal(recovered.error, undefined);
    assert.equal(recovered.signal, null);
    assert.equal(recovered.status, 0, recovered.stderr);
    assert.equal(recovered.stdout, '');
    assert.equal(JSON.parse(recovered.stderr).event, 'config.valid');
    assert.doesNotMatch(recovered.stderr, /canary/);
  });
}
