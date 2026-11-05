import assert from 'node:assert/strict';
import { execFile, execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { promisify } from 'node:util';
import { startInstalledSavedRemote, startInstalledSavedServer } from './installed-server.js';
import { createScenario, exchange } from './transport-scenario.js';

export async function verifyAccountManagement(consumer: string, env: NodeJS.ProcessEnv): Promise<void> {
  const state = mkdtempSync(join(consumer, 'account-management-'));
  const serverHome = join(state, 'server');
  const cli = join(consumer, 'node_modules/framerize/bin/framerize-server.js');
  const output: string[] = [];
  const runtimeEnv = { ...env, HOME: state, USERPROFILE: state, FRAMERIZE_SERVER_HOME: serverHome };
  let scenario: Awaited<ReturnType<typeof createScenario>> | undefined;
  let server: Awaited<ReturnType<typeof startInstalledSavedServer>> | undefined;
  let remote: Awaited<ReturnType<typeof startInstalledSavedRemote>> | undefined;
  const run = (args: string[], input = '', extra: NodeJS.ProcessEnv = {}) => {
    const text = execFileSync(process.execPath, [cli, ...args], { cwd: consumer,
      env: { ...runtimeEnv, ...extra },
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
    scenario = await createScenario('synthetic-saved-authorization');
    run(['configure', '--backend', 'http', '--upstream', scenario.url,
      '--authorization-env', 'FIXTURE_AUTHORIZATION', '--json'], '',
    { FIXTURE_AUTHORIZATION: 'Bearer synthetic-saved-authorization' });
    const configuration = run(['config', '--json']);
    assert.ok(configuration.includes(scenario.url));
    run(['check']);
    server = await startInstalledSavedServer(consumer, runtimeEnv);
    const remoteEnv = { ...runtimeEnv, FRM_HOME: join(state, 'remote') };
    const saved = await promisify(execFile)(process.execPath,
      [join(consumer, 'node_modules/framerize-remote/bin/frm.js'), 'setup',
        '--remote', server.url, '--port', '0', '--json'],
      { cwd: consumer, env: remoteEnv, timeout: 5000 });
    output.push(saved.stdout, saved.stderr);
    remote = await startInstalledSavedRemote(consumer, remoteEnv);
    for (const target of [server.url, remote.url]) {
      const reply = await exchange(target, '/v1/chat/completions', null,
        { body: { fixture: 'completed-json', stream: false, route: '/v1/chat/completions' } });
      assert.equal(reply.status, 200);
      assert.deepEqual(JSON.parse(reply.body.toString()), { done: true });
    }
    assert.deepEqual(scenario.state.failures, []);
    assert.ok(scenario.state.requests.some(request => request.authorization === 'Bearer synthetic-saved-authorization'));
    output.push(server.output(), remote.output());
    assert.doesNotMatch(output.join('\n'), /synthetic-(?:(?:first|second|rejected)-account|saved-authorization)/);
  } finally {
    const cleanup = await Promise.allSettled([remote?.close(), server?.close(), scenario?.close()]);
    rmSync(state, { recursive: true, force: true });
    for (const result of cleanup) if (result.status === 'rejected') throw result.reason;
  }
}
