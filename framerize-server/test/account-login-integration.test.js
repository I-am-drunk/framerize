import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, readFile, rm, stat } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runAccountsCommand } from '../dist/account-cli.js';

async function fixture(t) {
  const home = await mkdtemp(join(tmpdir(), 'account-login-integration-'));
  t.after(() => rm(home, { recursive: true, force: true }));
  let output = '';
  const deps = { env: { FRAMERIZE_SERVER_HOME: home },
    stdout: { write(text) { output += text; } } };
  return { home, deps, output: () => output,
    accounts: () => readFile(join(home, 'accounts.json'), 'utf8'),
    settings: () => readFile(join(home, 'server.json'), 'utf8') };
}

test('an invalid project is rejected before opening the browser or writing an account', async t => {
  const f = await fixture(t);
  let calls = 0;
  await assert.rejects(runAccountsCommand(['login', 'new', '--project-id', 'invalid/project'],
    { ...f.deps, login: async () => { calls += 1; return 'session=synthetic-unused-login'; } }));
  assert.equal(calls, 0);
  await assert.rejects(f.accounts(), { code: 'ENOENT' });
  await assert.rejects(f.settings(), { code: 'ENOENT' });
});

test('cancellation after sign-in leaves the previous files unchanged', async t => {
  const f = await fixture(t);
  const controller = new AbortController();
  let calls = 0;
  await assert.rejects(runAccountsCommand(['login', 'new', '--project-id', 'new-project'],
    { ...f.deps, signal: controller.signal, login: async signal => {
      assert.equal(signal, controller.signal);
      calls += 1;
      controller.abort();
      return 'session=synthetic-cancelled-login';
    } }));
  assert.equal(calls, 1);
  await assert.rejects(f.accounts(), { code: 'ENOENT' });
  await assert.rejects(f.settings(), { code: 'ENOENT' });
});

test('cancelled browser login preserves existing accounts and selected settings', async t => {
  const f = await fixture(t);
  await runAccountsCommand(['add', 'existing', '--project-id', 'existing-project',
    '--session-stdin', '--select', '--json'], { ...f.deps,
    readStdin: async () => 'session=synthetic-existing-login' });
  const before = [await f.accounts(), await f.settings()];
  const cancelled = new Error('Sign-in was cancelled.');
  await assert.rejects(runAccountsCommand(['login', 'new', '--project-id', 'new-project'],
    { ...f.deps, login: async () => { throw cancelled; } }), error => error === cancelled);
  assert.deepEqual([await f.accounts(), await f.settings()], before);
  assert.doesNotMatch(f.output(), /synthetic-existing-login/);
});

test('interactive browser login saves and selects the account without displaying its session', async t => {
  const f = await fixture(t);
  const events = [];
  const answers = ['personal', 'project-personal'];
  const result = await runAccountsCommand(['login', '--json'], { ...f.deps,
    ask: async () => { events.push('question'); return answers.shift(); },
    login: async () => { events.push('login'); return 'session=synthetic-browser-login'; } });
  assert.equal(result, 0);
  assert.deepEqual(events, ['question', 'question', 'login']);
  const accounts = JSON.parse(await f.accounts());
  assert.equal(accounts[0].id, 'personal');
  assert.equal(accounts[0].session, 'session=synthetic-browser-login');
  assert.equal(JSON.parse(await f.settings()).config.accountId, 'personal');
  assert.doesNotMatch(f.output(), /synthetic-browser-login/);
  if (process.platform !== 'win32') {
    assert.equal((await stat(join(f.home, 'accounts.json'))).mode & 0o777, 0o600);
  }
});
