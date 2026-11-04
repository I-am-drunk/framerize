import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageRoot = process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
const launcher = join(packageRoot, 'bin/framerize-remote.js');
const canary = 'private-setup-canary';
const origin = ['--remote', 'https://server.invalid'];
const server = ['--server-token-env', 'SETUP_SERVER'];
const valid = [...origin, ...server, '--local-token-env', 'SETUP_LOCAL'];
const cases = [
  ['missing remote', [], 'remote_required'],
  ['remote path', ['--remote', `https://${canary}.invalid/v1`], 'remote_invalid'],
  ['insecure remote', ['--remote', `http://${canary}.invalid`], 'remote_invalid'],
  ['missing server reference', origin, 'server_reference'],
  ['conflicting server references', [...origin, ...server, '--server-token-file', canary], 'server_reference'],
  ['missing local reference', [...origin, ...server], 'local_reference'],
  ['conflicting local references', [...valid, '--local-token-file', canary], 'local_reference'],
  ['invalid port', [...valid, '--port', canary], 'listen_invalid'],
  ['oversized port', [...valid, '--port', '65536'], 'listen_invalid'],
  ['invalid timeout', [...valid, '--inference-timeout-ms', canary], 'inference_invalid'],
  ['oversized timeout', [...valid, '--inference-timeout-ms', '2147463648'], 'inference_invalid'],
  ['zero timeout', [...valid, '--inference-timeout-ms', '0'], 'inference_invalid'],
  ['invalid host', [...valid, '--host', `${canary}/path`], 'listen_invalid'],
  ['missing exposure consent', [...valid, '--host', '0.0.0.0'], 'exposure_consent'],
  ['invalid server variable', [...origin, '--server-token-env', `${canary}-variable`,
    '--local-token-env', 'SETUP_LOCAL'], 'server_reference'],
  ['invalid local variable', [...origin, ...server, '--local-token-env', `${canary}-variable`], 'local_reference'],
  ['unknown option', [...valid, `--${canary}`], 'setup_arguments'],
  ['duplicate option', [...valid, ...origin], 'setup_arguments'],
  ['missing option value', [...valid, '--port'], 'setup_arguments'],
  ['repeated profile option without a value', [...valid, '--config'], 'setup_arguments'],
  ['duplicate profile path', [...valid, '--config', canary], 'setup_arguments'],
];

test('check explains saved exposure consent with a supported recovery command', async t => {
  const root = await mkdtemp(join(tmpdir(), 'remote-saved-consent-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const profile = join(root, 'profile.json');
  const saved = JSON.stringify({ remote: 'https://server.invalid', host: '0.0.0.0',
    serverToken: { kind: 'env', name: 'UNSET_SERVER' },
    localToken: { kind: 'env', name: 'UNSET_LOCAL' } });
  await writeFile(profile, saved, { mode: 0o600 });
  const options = { cwd: root, encoding: 'utf8', timeout: 5000,
    env: { HOME: root, USERPROFILE: root, TMPDIR: root,
      ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) } };
  for (const json of [false, true]) {
    const result = spawnSync(process.execPath,
      [launcher, 'check', '--config', profile, ...(json ? ['--json'] : [])], options);
    assert.equal(result.error, undefined);
    assert.equal(result.status, 1);
    assert.equal(result.stdout, '');
    if (json) assert.equal(JSON.parse(result.stderr).error, 'exposure_consent');
    assert.match(result.stderr, /framerize-remote setup --help/);
    assert.doesNotMatch(result.stderr, /UNSET_SERVER|UNSET_LOCAL/);
  }
  const help = spawnSync(process.execPath, [launcher, 'setup', '--help'], options);
  assert.equal(help.error, undefined);
  assert.equal(help.status, 0);
  assert.match(help.stdout, /--allow-non-loopback/);
  assert.equal(await readFile(profile, 'utf8'), saved);
  assert.deepEqual(await readdir(root), ['profile.json']);
});

test('serve host overrides explain their separate exposure consent', async t => {
  const root = await mkdtemp(join(tmpdir(), 'remote-serve-consent-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const profile = join(root, 'profile.json');
  const saved = JSON.stringify({ remote: 'https://server.invalid', host: '0.0.0.0',
    allowNonLoopback: true, serverToken: { kind: 'env', name: 'UNSET_SERVER' },
    localToken: { kind: 'env', name: 'UNSET_LOCAL' } });
  await writeFile(profile, saved, { mode: 0o600 });
  for (const json of [false, true]) {
    const result = spawnSync(process.execPath, [launcher, 'serve', '--config', profile,
      '--host', '0.0.0.0', ...(json ? ['--json'] : [])], {
      cwd: root, encoding: 'utf8', timeout: 5000,
      env: { HOME: root, USERPROFILE: root, TMPDIR: root,
        ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) },
    });
    assert.equal(result.error, undefined);
    assert.equal(result.status, 1);
    assert.equal(result.stdout, '');
    if (json) assert.equal(JSON.parse(result.stderr).error, 'exposure_consent');
    assert.match(result.stderr, /--allow-non-loopback to serve/);
    assert.doesNotMatch(result.stderr, /UNSET_SERVER|UNSET_LOCAL/);
    assert.equal(await readFile(profile, 'utf8'), saved);
    assert.deepEqual(await readdir(root), ['profile.json']);
  }
});

async function runSetup(root, args, json = false) {
  return spawnSync(process.execPath, [launcher, 'setup', ...args, ...(json ? ['--json'] : [])], {
    cwd: root, encoding: 'utf8', timeout: 5000,
    env: { HOME: root, USERPROFILE: root, TMPDIR: root,
      ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) },
  });
}

test('setup maps malformed global options to setup_arguments without touching the profile', async () => {
  const root = await mkdtemp(join(tmpdir(), 'remote-setup-globals-'));
  const profile = join(root, 'profile.json');
  const tail = ['--remote', 'https://server.invalid', '--server-token-env', 'SETUP_SERVER',
    '--local-token-env', 'SETUP_LOCAL'];
  const globals = [
    ['missing global profile path', ['--config', ...tail]],
    ['profile flag followed by another flag', ['--config', '--remote', 'https://server.invalid', ...tail.slice(2)]],
    ['duplicate global json flag', ['--config', profile, '--json', '--json', ...tail]],
  ];
  try {
    await writeFile(profile, 'existing-profile', { mode: 0o600 });
    for (const [name, args] of globals) {
      const jsonModes = args.includes('--json') ? [true] : [false, true];
      for (const json of jsonModes) {
        const result = await runSetup(root, args, json);
        assert.equal(result.error, undefined, name);
        assert.equal(result.status, 1, name);
        assert.equal(result.stdout, '', name);
        assert.doesNotMatch(result.stderr, /private-setup-canary|existing-profile/, name);
        if (json) assert.equal(JSON.parse(result.stderr).error, 'setup_arguments', name);
        else assert.match(result.stderr, /^Not ready: .+\nNext: .+\n$/, name);
        assert.equal(await readFile(profile, 'utf8'), 'existing-profile', name);
      }
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});

for (const [command, args] of [['check', ['--config', '--json']], ['serve', ['--config']]]) {
  test(`${command} keeps malformed global options as command_failed`, async () => {
    const root = await mkdtemp(join(tmpdir(), 'remote-global-failure-'));
    try {
      const result = spawnSync(process.execPath, [launcher, command, ...args], {
        cwd: root, encoding: 'utf8', timeout: 5000,
        env: { HOME: root, USERPROFILE: root, TMPDIR: root,
          ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) },
      });
      assert.equal(result.error, undefined);
      assert.equal(result.status, 1);
      assert.equal(result.stdout, '');
      assert.doesNotMatch(result.stderr, /setup_arguments/);
      if (args.includes('--json')) assert.equal(JSON.parse(result.stderr).error, 'command_failed');
      else assert.match(result.stderr, /^framerize-remote failed to run the requested command\n/);
    } finally { await rm(root, { recursive: true, force: true }); }
  });
}

test('setup explains an unwritable profile target without exposing its path', async () => {
  const root = await mkdtemp(join(tmpdir(), `${canary}-`));
  try {
    const result = spawnSync(process.execPath, [launcher, 'setup', '--config', root, ...valid, '--json'], {
      encoding: 'utf8', timeout: 5000,
      env: { HOME: root, USERPROFILE: root, TMPDIR: root,
        ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) },
    });
    assert.equal(result.error, undefined);
    assert.equal(result.status, 1);
    assert.equal(result.stdout, '');
    assert.equal(JSON.parse(result.stderr).error, 'profile_write');
    assert.doesNotMatch(result.stderr, /private-setup-canary/);
    assert.deepEqual(await readdir(root), []);
  } finally { await rm(root, { recursive: true, force: true }); }
});

for (const [name, args, code] of cases) {
  test(`setup explains ${name} without changing the profile`, async () => {
    const root = await mkdtemp(join(tmpdir(), 'remote-setup-errors-'));
    const profile = join(root, 'profile.json');
    try {
      await writeFile(profile, 'existing-profile', { mode: 0o600 });
      for (const json of [false, true]) {
        const result = spawnSync(process.execPath, [launcher, 'setup', '--config', profile,
          ...args, ...(json ? ['--json'] : [])], {
          cwd: root, encoding: 'utf8', timeout: 5000,
          env: { HOME: root, USERPROFILE: root, TMPDIR: root,
            ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) },
        });
        assert.equal(result.error, undefined);
        assert.equal(result.status, 1);
        assert.equal(result.stdout, '');
        assert.doesNotMatch(result.stderr, /private-setup-canary|existing-profile/);
        if (json) assert.equal(JSON.parse(result.stderr).error, code);
        else assert.match(result.stderr, /^Not ready: .+\nNext: .+\n$/);
        assert.equal(await readFile(profile, 'utf8'), 'existing-profile');
        assert.deepEqual(await readdir(root), ['profile.json']);
      }
    } finally { await rm(root, { recursive: true, force: true }); }
  });
}
