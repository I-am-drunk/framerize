import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { join } from 'node:path';

export function verifyAccountManagement(consumer: string, env: NodeJS.ProcessEnv): void {
  const state = mkdtempSync(join(consumer, 'account-management-'));
  const serverHome = join(state, 'server');
  const cli = join(consumer, 'node_modules/framerize/bin/framerize-server.js');
  const output: string[] = [];
  const run = (args: string[], input = '', extra: NodeJS.ProcessEnv = {}) => {
    const text = execFileSync(process.execPath, [cli, ...args], { cwd: consumer,
      env: { ...env, HOME: state, USERPROFILE: state, FRAMERIZE_SERVER_HOME: serverHome, ...extra },
      input, encoding: 'utf8', timeout: 5000 });
    output.push(text);
    return text;
  };
  const list = () => JSON.parse(run(['accounts', 'list', '--json']));
  try {
    assert.deepEqual(list().accounts, []);
    run(['accounts', 'add', 'alpha', '--project-id', 'project-alpha', '--session-stdin', '--select', '--json'],
      'session=synthetic-first-account\n');
    run(['accounts', 'add', 'beta', '--project-id', 'project-beta', '--session-stdin', '--json'],
      'session=synthetic-second-account\n');
    assert.equal(list().selected, 'alpha');
    assert.deepEqual(list().accounts.map((account: { id: string }) => account.id), ['alpha', 'beta']);
    assert.equal(JSON.parse(run(['accounts', 'show', 'alpha', '--json'])).session, 'saved');
    run(['accounts', 'use', 'beta', '--json']);
    assert.equal(list().selected, 'beta');
    run(['accounts', 'disable', 'beta', '--json']);
    assert.equal(list().selected, null);
    assert.equal(list().accounts.find((account: { id: string }) => account.id === 'beta').enabled, false);
    run(['accounts', 'enable', 'beta', '--json']);
    assert.equal(list().accounts.find((account: { id: string }) => account.id === 'beta').enabled, true);
    run(['accounts', 'remove', 'beta', '--json']);
    assert.deepEqual(list().accounts.map((account: { id: string }) => account.id), ['alpha']);
    const accountsPath = join(serverHome, 'accounts.json');
    if (process.platform !== 'win32') assert.equal(statSync(accountsPath).mode & 0o777, 0o600);
    const settingsPath = join(serverHome, 'server.json');
    const before = [accountsPath, settingsPath].map(path => readFileSync(path));
    assert.throws(() => run(['accounts', 'add', 'invalid', '--project-id', 'invalid/project',
      '--session-stdin', '--select', '--json'], 'session=synthetic-rejected-account\n'));
    assert.deepEqual([accountsPath, settingsPath].map(path => readFileSync(path)), before);
    run(['config', '--json']);
    run(['configure', '--backend', 'http', '--upstream', 'http://127.0.0.1:9',
      '--authorization-env', 'FIXTURE_AUTHORIZATION', '--json'], '',
    { FIXTURE_AUTHORIZATION: 'Bearer synthetic-saved-authorization' });
    const configuration = run(['config', '--json']);
    assert.match(configuration, /http:\/\/127\.0\.0\.1:9/);
    run(['check']);
    assert.doesNotMatch(output.join('\n'), /synthetic-(?:(?:first|second|rejected)-account|saved-authorization)/);
  } finally {
    rmSync(state, { recursive: true, force: true });
  }
}
