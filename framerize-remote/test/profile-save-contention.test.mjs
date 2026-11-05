import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

async function api(platform) {
  const compiled = await build({
    entryPoints: [fileURLToPath(new URL('../src/configStore.ts', import.meta.url))],
    platform: 'node', format: 'esm', bundle: true, write: false,
    define: { 'process.platform': JSON.stringify(platform) },
  });
  return import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);
}
const windows = await api('win32');
const posix = await api('linux');
const config = { remote: 'http://127.0.0.1:8789', port: 0 };

async function fixture(t, replace) {
  const root = await fs.mkdtemp(join(tmpdir(), 'frm-profile-replace-'));
  const profile = join(root, 'profile.json');
  await fs.writeFile(profile, 'previous profile', { mode: 0o600 });
  const rename = fs.rename;
  let calls = 0;
  fs.rename = async (...args) => replace(++calls, rename, args, profile);
  syncBuiltinESMExports();
  t.after(async () => {
    fs.rename = rename; syncBuiltinESMExports();
    await fs.rm(root, { recursive: true, force: true });
  });
  return { root, profile, calls: () => calls };
}

for (const code of ['EPERM', 'EACCES', 'EBUSY']) {
  test(`a transient Windows ${code} replacement failure retains the old profile until retry`, async t => {
    const f = await fixture(t, async (attempt, rename, args, profile) => {
      assert.equal(await fs.readFile(profile, 'utf8'), 'previous profile');
      if (attempt === 1) throw Object.assign(new Error('Synthetic sharing violation'), { code });
      return rename(...args);
    });
    await windows.saveConfigFile(f.profile, config);
    assert.equal(f.calls(), 2);
    assert.equal(JSON.parse(await fs.readFile(f.profile, 'utf8')).remote, config.remote);
    assert.deepEqual(await fs.readdir(f.root), ['profile.json']);
  });
}

test('a non-transient Windows failure keeps the original profile without retry', async t => {
  const f = await fixture(t, async () => {
    throw Object.assign(new Error('Synthetic disk failure'), { code: 'ENOSPC' });
  });
  await assert.rejects(windows.saveConfigFile(f.profile, config), { code: 'profile_write' });
  assert.equal(f.calls(), 1);
  assert.equal(await fs.readFile(f.profile, 'utf8'), 'previous profile');
  assert.deepEqual(await fs.readdir(f.root), ['profile.json']);
});

test('persistent Windows sharing failures stop within a bounded number of attempts', async t => {
  const f = await fixture(t, async () => {
    throw Object.assign(new Error('Synthetic persistent sharing violation'), { code: 'EPERM' });
  });
  await assert.rejects(windows.saveConfigFile(f.profile, config), { code: 'profile_write' });
  assert.equal(f.calls(), 6);
  assert.equal(await fs.readFile(f.profile, 'utf8'), 'previous profile');
  assert.deepEqual(await fs.readdir(f.root), ['profile.json']);
});

test('replacement succeeds on the last Windows retry', async t => {
  const f = await fixture(t, async (attempt, rename, args, profile) => {
    assert.equal(await fs.readFile(profile, 'utf8'), 'previous profile');
    if (attempt < 6) throw Object.assign(new Error('Synthetic sharing violation'), { code: 'EBUSY' });
    return rename(...args);
  });
  await windows.saveConfigFile(f.profile, config);
  assert.equal(f.calls(), 6);
  assert.equal(JSON.parse(await fs.readFile(f.profile, 'utf8')).remote, config.remote);
  assert.deepEqual(await fs.readdir(f.root), ['profile.json']);
});

test('POSIX permission errors retain their immediate failure behavior', async t => {
  const f = await fixture(t, async () => {
    throw Object.assign(new Error('Synthetic POSIX permission failure'), { code: 'EPERM' });
  });
  await assert.rejects(posix.saveConfigFile(f.profile, config), { code: 'profile_write' });
  assert.equal(f.calls(), 1);
  assert.equal(await fs.readFile(f.profile, 'utf8'), 'previous profile');
  assert.deepEqual(await fs.readdir(f.root), ['profile.json']);
});
