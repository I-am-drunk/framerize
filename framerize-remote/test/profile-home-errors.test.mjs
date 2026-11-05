import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageRoot = process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
const launcher = join(packageRoot, 'bin/framerize-remote.js');
const frmLauncher = join(packageRoot, 'bin/frm.js');
const canary = 'private-profile-home-canary';
const serverReference = 'PROFILE_HOME_UNSET_SERVER';
const localReference = 'PROFILE_HOME_UNSET_LOCAL';
const setupArgs = ['setup', '--remote', 'https://server.invalid',
  '--server-token-env', serverReference, '--local-token-env', localReference,
  '--host', '127.0.0.1', '--port', '0', '--inference-timeout-ms', '5000'];
const expectedDiagnostic = {
  error: 'profile_home_invalid', code: 'profile_home_invalid',
  message: 'FRM_HOME, or FRAMERIZE_REMOTE_HOME when FRM_HOME is unset, must be an absolute directory path without control characters.',
  next: 'Set FRM_HOME to an absolute directory without control characters, or unset both home variables to use the default.',
};
const invalidHomes = [
  ['empty', () => ''],
  ['relative', () => canary],
  ['control character', root => join(root, `${canary}\u0001`)],
];
const expectedProfile = {
  remote: 'https://server.invalid',
  serverToken: { kind: 'env', name: serverReference },
  localToken: { kind: 'env', name: localReference },
  host: '127.0.0.1', port: 0, inferenceTimeoutMs: 5000,
  allowInsecureRemote: false, allowNonLoopback: false,
};

async function privateState(context) {
  const root = await mkdtemp(join(tmpdir(), `${canary}-`));
  context.after(() => rm(root, { recursive: true, force: true }));
  await writeFile(join(root, 'unchanged.txt'), canary, { mode: 0o600 });
  return root;
}

function launch(root, args, overrides = {}, program = launcher) {
  return spawnSync(process.execPath, [program, ...args], {
    cwd: root, encoding: 'utf8', timeout: 5000, killSignal: 'SIGKILL', maxBuffer: 64 * 1024,
    env: { HOME: root, USERPROFILE: root, TMPDIR: root, TEMP: root, TMP: root,
      ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}),
      ...overrides },
  });
}

function assertFinished(result, status) {
  assert.equal(result.error, undefined, 'launcher must finish within its timeout');
  assert.equal(result.signal, null, 'launcher must exit without a signal');
  assert.equal(result.status, status);
}

function assertRedacted(result, root, invalid = '') {
  const output = result.stdout + result.stderr;
  for (const value of [root, canary, serverReference, localReference, invalid].filter(Boolean)) {
    assert.equal(output.includes(value), false, 'output must not expose private state or references');
  }
}

async function snapshot(root, prefix = '') {
  const result = [];
  const entries = await readdir(root, { withFileTypes: true });
  entries.sort((first, second) => first.name.localeCompare(second.name));
  for (const entry of entries) {
    const path = join(root, entry.name);
    const name = join(prefix, entry.name);
    if (entry.isDirectory()) {
      result.push([name, null], ...await snapshot(path, name));
    } else {
      assert.equal(entry.isFile(), true, 'private state must contain only regular files');
      result.push([name, await readFile(path, 'utf8')]);
    }
  }
  return result;
}

for (const [name, configured] of invalidHomes) {
  test(`help and version remain accessible with ${name} profile home`,
    { timeout: 25000 }, async context => {
      const root = await privateState(context);
      const before = await snapshot(root);
      const invalid = configured(root);
      for (const args of [['help'], ['setup', '--help'], ['serve', '--help'], ['--version']]) {
        const result = launch(root, args, { FRAMERIZE_REMOTE_HOME: invalid });
        assertFinished(result, 0);
        assert.equal(result.stderr, '');
        assertRedacted(result, root, invalid);
        if (args[0] === '--version') assert.match(result.stdout, /^\d+\.\d+\.\d+(?:[+-][^\s]+)?\n$/);
        else assert.match(result.stdout, /\bfrm\b/);
        assert.deepEqual(await snapshot(root), before);
      }
    });
}

for (const choice of ['absolute', 'default', 'explicit']) {
  test(`setup recovers through ${choice} profile selection without credentials`,
    { timeout: 15000 }, async context => {
      const root = await privateState(context);
      const before = await snapshot(root);
      const failed = launch(root, [...setupArgs, '--json'], { FRAMERIZE_REMOTE_HOME: canary });
      assertFinished(failed, 1);
      assert.equal(failed.stdout, '');
      assertRedacted(failed, root, canary);
      assert.deepEqual(await snapshot(root), before);
      const directory = choice === 'default' ? '.framerize-remote' : choice;
      const profile = join(root, directory, 'framerize-remote.json');
      const overrides = choice === 'default' ? {}
        : { FRAMERIZE_REMOTE_HOME: choice === 'absolute' ? join(root, directory) : canary };
      const args = choice === 'explicit' ? ['--config', profile] : [];
      const result = launch(root, [...setupArgs, ...args, '--json'], overrides);
      assertFinished(result, 0);
      assert.equal(result.stderr, '');
      assert.equal(result.stdout, '{"saved":true}\n');
      assertRedacted(result, root, canary);
      assert.deepEqual(JSON.parse(await readFile(profile, 'utf8')), expectedProfile);
      const after = await snapshot(root);
      assert.deepEqual(after.map(([path]) => path).sort(),
        [directory, relative(root, profile), 'unchanged.txt'].sort());
      assert.equal(await readFile(join(root, 'unchanged.txt'), 'utf8'), canary);
    });
}

for (const command of ['setup', 'check', 'serve']) {
  for (const [name, configured] of invalidHomes) {
    for (const json of [false, true]) {
      test(`${command} diagnoses ${name} profile home in ${json ? 'JSON' : 'human'} output`,
        { timeout: 10000 }, async context => {
          const root = await privateState(context);
          const before = await snapshot(root);
          const invalid = configured(root);
          const args = command === 'setup' ? setupArgs : [command];
          const result = launch(root, [...args, ...(json ? ['--json'] : [])],
            { FRAMERIZE_REMOTE_HOME: invalid });
          assertFinished(result, 1);
          assert.equal(result.stdout, '');
          assertRedacted(result, root, invalid);
          assert.deepEqual(await snapshot(root), before);
          if (json) assert.deepEqual(JSON.parse(result.stderr), expectedDiagnostic);
          else assert.equal(result.stderr,
            `Not ready: ${expectedDiagnostic.message}\nNext: ${expectedDiagnostic.next}\n`);
        });
    }
  }
}

for (const [name, configured] of invalidHomes) for (const json of [false, true]) {
  test(`frm setup diagnoses ${name} primary home in ${json ? 'JSON' : 'human'} output`,
    { timeout: 10000 }, async context => {
      const root = await privateState(context), invalid = configured(root);
      const before = await snapshot(root);
      const result = launch(root, [...setupArgs, ...(json ? ['--json'] : [])], {
        FRM_HOME: invalid, FRAMERIZE_REMOTE_HOME: join(root, 'valid-fallback'),
      }, frmLauncher);
      assertFinished(result, 1);
      assert.equal(result.stdout, '');
      assertRedacted(result, root, invalid);
      assert.deepEqual(await snapshot(root), before);
      if (json) assert.deepEqual(JSON.parse(result.stderr), expectedDiagnostic);
      else assert.equal(result.stderr,
        `Not ready: ${expectedDiagnostic.message}\nNext: ${expectedDiagnostic.next}\n`);
    });
}

test('frm setup uses a valid primary home despite an invalid fallback',
  { timeout: 10000 }, async context => {
    const root = await privateState(context);
    const directory = 'primary';
    const profile = join(root, directory, 'framerize-remote.json');
    const result = launch(root, [...setupArgs, '--json'], {
      FRM_HOME: join(root, directory), FRAMERIZE_REMOTE_HOME: canary,
    }, frmLauncher);
    assertFinished(result, 0);
    assert.equal(result.stdout, '{"saved":true}\n');
    assert.equal(result.stderr, '');
    assertRedacted(result, root, canary);
    assert.deepEqual(JSON.parse(await readFile(profile, 'utf8')), expectedProfile);
    assert.deepEqual((await snapshot(root)).map(([path]) => path).sort(),
      [directory, relative(root, profile), 'unchanged.txt'].sort());
    assert.equal(await readFile(join(root, 'unchanged.txt'), 'utf8'), canary);
  });
