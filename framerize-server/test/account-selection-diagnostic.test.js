import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const launcher = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ? join(process.env.FRAMERIZE_SERVER_PACKAGE_ROOT, 'bin/framerize-server.js')
  : fileURLToPath(new URL('../bin/framerize-server.js', import.meta.url));
const account = { id: 'account-id-canary', projectId: 'project-canary',
  session: { env: 'DIAGNOSTIC_SESSION' } };
const cases = [
  ['ambiguous', [account, { ...account, id: 'second-id-canary' }], undefined],
  ['unknown', [account], 'unknown-id-canary'],
  ['disabled', [{ ...account, enabled: false }], account.id],
  ['unconfigured', [{ id: account.id }], account.id],
  ['empty', [], undefined],
];

for (const [name, accounts, accountId] of cases) for (const command of ['check', 'serve']) {
  test(`${command} explains and recovers from ${name} account selection`, context => {
    const root = mkdtempSync(join(tmpdir(), 'server-selection-diagnostic-'));
    context.after(() => rmSync(root, { recursive: true, force: true }));
    const file = join(root, 'private-path-canary');
    writeFileSync(file, JSON.stringify(accounts));
    const invoke = (command, id = accountId) => spawnSync(process.execPath,
      [launcher, command, '--backend', 'framer', '--accounts-file', file,
        '--server-token-env', 'DIAGNOSTIC_TOKEN', '--framer-origin', 'http://127.0.0.1:1',
        '--host', '127.0.0.1', '--port', '0', ...(id ? ['--account-id', id] : [])], {
        cwd: root, encoding: 'utf8', timeout: 5000,
        env: { HOME: root, USERPROFILE: root, TMPDIR: root,
          DIAGNOSTIC_TOKEN: 'synthetic-token-canary',
          DIAGNOSTIC_SESSION: 'session=synthetic-session-canary',
          NODE_DISABLE_COMPILE_CACHE: '1',
          ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) },
      });
    const result = invoke(command);
    assert.equal(result.error, undefined);
    assert.equal(result.signal, null);
    assert.equal(result.status, 1);
    assert.equal(result.stdout, '');
    const diagnostic = JSON.parse(result.stderr);
    assert.equal(diagnostic.event, 'server.failed');
    assert.equal(diagnostic.message, 'Select one configured Framer account.');
    assert.match(diagnostic.next, /--accounts-file/);
    assert.match(diagnostic.next, /--account-id/);
    assert.match(diagnostic.next, /enabled.*projectId.*session reference/);
    assert.doesNotMatch(result.stderr, /canary|private-path/);
    if (!['ambiguous', 'unknown'].includes(name)) {
      writeFileSync(file, JSON.stringify([account]));
    }
    const recovered = invoke('check', account.id);
    assert.equal(recovered.error, undefined);
    assert.equal(recovered.signal, null);
    assert.equal(recovered.status, 0, recovered.stderr);
    assert.equal(recovered.stdout, '');
    assert.equal(JSON.parse(recovered.stderr).event, 'config.valid');
    assert.doesNotMatch(recovered.stderr, /canary|private-path/);
  });
}
