import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, mkdir, readFile, writeFile, rm, access, realpath } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve, delimiter } from 'node:path';
import { fileURLToPath } from 'node:url';

const execute = promisify(execFile);
const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));

const posix = {
  skip: process.platform === 'win32' && 'Git-root local settings apply on POSIX', timeout: 20000,
};

async function fixture(t, linked = false) {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'frm-local-settings-')));
  t.after(() => rm(root, { recursive: true, force: true }));
  const home = join(root, 'home'), bin = join(home, '.local/bin');
  const repo = join(root, 'repo with trailing space '), checkout = join(root, 'linked checkout ');
  const cwd = join(linked ? checkout : repo, 'packages/app');
  const profileHome = join(root, 'profile'), profile = join(profileHome, 'framerize-remote.json');
  const record = join(root, 'client.json'), client = join(bin, 'claude'), guard = join(root, 'guard.mjs');
  await Promise.all([bin, profileHome, join(repo, '.claude')]
    .map(path => mkdir(path, { recursive: true })));
  const env = { HOME: home, USERPROFILE: home, FRM_HOME: profileHome,
    PATH: `${bin}${delimiter}${process.env.PATH ?? ''}`, FRM_TEST_CLIENT_RECORD: record,
    GIT_CONFIG_NOSYSTEM: '1', GIT_CONFIG_GLOBAL: '/dev/null' };
  await execute('git', ['init', '--quiet', repo], { env, timeout: 5000 });
  if (linked) {
    await execute('git', ['-C', repo, '-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid',
      'commit', '--quiet', '--allow-empty', '-m', 'Synthetic worktree fixture'], { env, timeout: 5000 });
    await execute('git', ['-C', repo, 'worktree', 'add', '--quiet', '--detach', checkout], { env, timeout: 5000 });
  }
  await mkdir(join(cwd, '.claude'), { recursive: true });
  await writeFile(client, `#!${process.execPath}
const fs = require('node:fs');
fs.writeFileSync(process.env.FRM_TEST_CLIENT_RECORD, JSON.stringify({
  args: process.argv.slice(2), token: process.env.ANTHROPIC_AUTH_TOKEN,
  base: process.env.ANTHROPIC_BASE_URL,
}));
`, { mode: 0o700 });
  await writeFile(guard, `import processes from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
const spawn = processes.spawn;
processes.spawn = (file, args, options) => {
  if (file !== 'git' && file !== ${JSON.stringify(client)}) throw new Error('Unexpected command');
  return spawn(file, args, options);
};
globalThis.fetch = () => { throw new Error('Unexpected network request'); };
syncBuiltinESMExports();
`);
  const original = JSON.stringify({ remote: 'http://127.0.0.1:9', port: 8788,
    localToken: { kind: 'value', value: 'synthetic-local-token' } });
  await writeFile(profile, original, { mode: 0o600 });
  const local = join(repo, '.claude/settings.local.json');
  const invoke = async () => {
    try {
      const output = await execute(process.execPath, ['--import', guard,
        join(packageRoot, 'bin/frm.js'), 'client', 'claude'], { cwd, env, timeout: 8000, maxBuffer: 65536 });
      return { code: 0, ...output };
    } catch (error) {
      if (typeof error.code !== 'number') throw error;
      return { code: error.code, stdout: error.stdout, stderr: error.stderr };
    }
  };
  return { local, nested: join(cwd, '.claude/settings.local.json'), record, profile, original, invoke };
}

test('a nested launcher checks the Git-root local file before starting Claude', posix, async t => {
  const { local, record, profile, original, invoke } = await fixture(t);
  await writeFile(local, '{"env":{"ANTHROPIC_AUTH_TOKEN":"private-canary"}}');
  const blocked = await invoke();
  assert.equal(blocked.code, 1);
  assert.equal(blocked.stdout, '');
  assert.ok(blocked.stderr.includes(`${local}: env.ANTHROPIC_AUTH_TOKEN`));
  assert.ok(!blocked.stderr.includes('private-canary'));
  await assert.rejects(access(record), { code: 'ENOENT' });
  assert.equal(await readFile(profile, 'utf8'), original);

  await writeFile(local, '{"env":{"ANTHROPIC_BASE_URL":"https://other.example"}}');
  const launched = await invoke();
  assert.equal(launched.code, 0, launched.stderr);
  assert.equal(launched.stderr, '');
  const child = JSON.parse(await readFile(record, 'utf8'));
  assert.deepEqual(child.args, ['--settings', '{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:8788/anthropic"}}']);
  assert.equal(child.token, 'synthetic-local-token');
  assert.equal(child.base, 'http://127.0.0.1:8788/anthropic');
  assert.ok(!child.args.join(' ').includes(child.token));
  assert.equal(await readFile(profile, 'utf8'), original);
});

for (const [key, settings] of [
  ['apiKeyHelper', { apiKeyHelper: 'nested-private-canary' }],
  ['env.ANTHROPIC_AUTH_TOKEN', { env: { ANTHROPIC_AUTH_TOKEN: 'nested-private-canary' } }],
]) {
  test(`a legacy nested credential ${key} blocks alongside root settings`, posix, async t => {
    const { local, nested, record, profile, original, invoke } = await fixture(t);
    await writeFile(local, '{"model":"opus"}');
    await writeFile(nested, JSON.stringify(settings));
    const blocked = await invoke();
    assert.equal(blocked.code, 1);
    assert.equal(blocked.stdout, '');
    assert.ok(blocked.stderr.includes(`${nested}: ${key}`));
    assert.ok(!blocked.stderr.includes('nested-private-canary'));
    await assert.rejects(access(record), { code: 'ENOENT' });
    assert.equal(await readFile(profile, 'utf8'), original);
    await rm(nested);
    assert.equal((await invoke()).code, 0);
    assert.equal(JSON.parse(await readFile(record, 'utf8')).token, 'synthetic-local-token');
    assert.equal(await readFile(profile, 'utf8'), original);
  });
}

test('a linked worktree checks the main checkout local file', posix, async t => {
  const { local, record, profile, original, invoke } = await fixture(t, true);
  await writeFile(local, '{"apiKeyHelper":"main-private-canary"}');
  const blocked = await invoke();
  assert.equal(blocked.code, 1);
  assert.equal(blocked.stdout, '');
  assert.ok(blocked.stderr.includes(`${local}: apiKeyHelper`));
  assert.ok(!blocked.stderr.includes('main-private-canary'));
  await assert.rejects(access(record), { code: 'ENOENT' });
  assert.equal(await readFile(profile, 'utf8'), original);
  await writeFile(local, '{"env":{"ANTHROPIC_BASE_URL":"https://other.example"}}');
  const launched = await invoke();
  assert.equal(launched.code, 0, launched.stderr);
  assert.equal(launched.stderr, '');
  const child = JSON.parse(await readFile(record, 'utf8'));
  assert.deepEqual(child.args, ['--settings', '{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:8788/anthropic"}}']);
  assert.equal(child.token, 'synthetic-local-token');
  assert.ok(!child.args.join(' ').includes(child.token));
  assert.equal(await readFile(profile, 'utf8'), original);
});
