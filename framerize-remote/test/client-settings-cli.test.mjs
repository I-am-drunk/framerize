import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, mkdir, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const execute = promisify(execFile);
const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const guard = new URL('./helpers/settings-cli-preload.mjs', import.meta.url).href;

test('CLI keeps undo usable across working directories and settings targets', { timeout: 15000 }, async t => {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'frm-settings-cli-')));
  t.after(() => rm(root, { recursive: true, force: true }));
  const first = join(root, 'first'), second = join(root, 'second'), state = join(root, 'state');
  await Promise.all([first, second, state].map(path => mkdir(path)));
  const token = 'synthetic-cli-settings-token';
  const profilePath = join(state, 'framerize-remote.json');
  const profile = JSON.stringify({ remote: 'http://127.0.0.1:1', port: 8788,
    localToken: { kind: 'value', value: token } });
  await writeFile(profilePath, profile, { mode: 0o600 });
  const env = { HOME: root, USERPROFILE: root, FRM_HOME: state, CLAUDE_CONFIG_DIR: 'config',
    PATH: '', NODE_DISABLE_COMPILE_CACHE: '1',
    ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) };
  const invoke = async (cwd, option) => {
    let result;
    try { result = { code: 0, ...await execute(process.execPath, ['--import', guard,
      join(packageRoot, 'bin/frm.js'), 'client', 'claude', option],
    { cwd, env, timeout: 5000, maxBuffer: 65536 }) }; }
    catch (error) {
      if (typeof error.code !== 'number') throw error;
      result = { code: error.code, stdout: error.stdout, stderr: error.stderr };
    }
    assert.ok(!`${result.stdout}${result.stderr}`.includes(token), 'CLI output must not disclose its token');
    return result;
  };
  const firstFile = join(first, 'config/settings.json'), secondFile = join(second, 'config/settings.json');
  const recordPath = join(state, 'claude-settings.json');
  const firstPersist = await invoke(first, '--persist');
  assert.equal(firstPersist.code, 0, firstPersist.stderr);
  const saved = await readFile(firstFile, 'utf8'), record = await readFile(recordPath, 'utf8');
  assert.equal(JSON.parse(record).settingsPath, firstFile);
  const refused = await invoke(second, '--persist');
  assert.equal(refused.code, 1);
  assert.match(refused.stderr, /--undo/);
  assert.equal(await readFile(firstFile, 'utf8'), saved);
  assert.equal(await readFile(recordPath, 'utf8'), record);
  await assert.rejects(readFile(secondFile), { code: 'ENOENT' });
  const undone = await invoke(second, '--undo');
  assert.equal(undone.code, 0, undone.stderr);
  assert.match(undone.stdout, /Removed the Remote values/);
  assert.deepEqual(JSON.parse(await readFile(firstFile, 'utf8')), {});
  await assert.rejects(readFile(recordPath), { code: 'ENOENT' });
  const secondPersist = await invoke(second, '--persist');
  assert.equal(secondPersist.code, 0, secondPersist.stderr);
  assert.equal(JSON.parse(await readFile(secondFile, 'utf8')).env.ANTHROPIC_AUTH_TOKEN, token);
  const undoneSecond = await invoke(first, '--undo');
  assert.equal(undoneSecond.code, 0, undoneSecond.stderr);
  assert.match(undoneSecond.stdout, /Removed the Remote values/);
  assert.deepEqual(JSON.parse(await readFile(secondFile, 'utf8')), {});
  await assert.rejects(readFile(recordPath), { code: 'ENOENT' });
  assert.equal(await readFile(profilePath, 'utf8'), profile);
});
