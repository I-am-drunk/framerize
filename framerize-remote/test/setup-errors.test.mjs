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
  ['missing profile path', [...valid, '--config'], 'setup_arguments'],
  ['duplicate profile path', [...valid, '--config', canary], 'setup_arguments'],
];

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
