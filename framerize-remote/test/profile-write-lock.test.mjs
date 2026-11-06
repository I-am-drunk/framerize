import assert from 'node:assert/strict';
import test from 'node:test';
import { access, mkdir, mkdtemp, readFile, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { withProfileLock } from './profileLock.ts'; export { runSetup } from './cmdSetup.ts';",
  resolveDir: fileURLToPath(new URL('../src/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false });
stop();
const { withProfileLock, runSetup } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const args = ['--remote', 'http://127.0.0.1:8789'];
const missing = path => assert.rejects(access(path), { code: 'ENOENT' });
async function fixture(context) {
  const root = await mkdtemp(join(tmpdir(), 'frm-profile-lock-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  return { root, profile: join(root, 'remote.json'), lock: join(root, '.remote.json.lock') };
}

test('directory aliases share a lock while different profiles remain independent', async context => {
  const { root, profile, lock } = await fixture(context);
  const real = join(root, 'real'), alias = join(root, 'alias');
  await mkdir(real);
  await symlink(real, alias, 'junction');
  await withProfileLock(join(real, 'profile.json'), async () => {
    const held = join(real, '.profile.json.lock');
    assert.equal(await readFile(held, 'utf8'), `${process.pid}\n`);
    if (process.platform !== 'win32') assert.equal((await stat(held)).mode & 0o777, 0o600);
    await assert.rejects(runSetup(args, join(alias, 'profile.json'), {}), { code: 'profile_busy' });
    await missing(join(real, 'profile.json'));
    await runSetup(args, profile, {});
    await missing(lock);
  });
  await missing(join(real, '.profile.json.lock'));
  await runSetup(args, join(alias, 'profile.json'), {});
  assert.equal(JSON.parse(await readFile(join(real, 'profile.json'), 'utf8')).remote, args[1]);
});

test('operation errors release the lock and preserve the original error', async context => {
  const { profile, lock } = await fixture(context);
  const reason = new Error('synthetic operation failure');
  await assert.rejects(withProfileLock(profile, async () => { throw reason; }), error => error === reason);
  await missing(lock);
  await runSetup(args, profile, {});
  assert.equal(JSON.parse(await readFile(profile, 'utf8')).remote, args[1]);
  await missing(lock);
});

test('a failed profile save releases its lock so a corrected path can retry', async context => {
  const { profile, lock } = await fixture(context);
  await mkdir(profile);
  await assert.rejects(runSetup(args, profile, {}), { code: 'profile_write' });
  await missing(lock);
  await rm(profile, { recursive: true });
  await runSetup(args, profile, {});
  assert.equal(JSON.parse(await readFile(profile, 'utf8')).remote, args[1]);
  await missing(lock);
});

test('an existing lock is preserved until the owner removes it', async context => {
  const { profile, lock } = await fixture(context);
  await writeFile(lock, 'synthetic owner\n', { mode: 0o600 });
  await assert.rejects(runSetup(args, profile, {}), { code: 'profile_busy' });
  assert.equal(await readFile(lock, 'utf8'), 'synthetic owner\n');
  await missing(profile);
  await rm(lock);
  await runSetup(args, profile, {});
  await missing(lock);
});

test('an invalid parent path reports a write error without replacing it', async context => {
  const { root } = await fixture(context);
  const parent = join(root, 'not-a-directory');
  await writeFile(parent, 'keep this file\n');
  await assert.rejects(runSetup(args, join(parent, 'remote.json'), {}), { code: 'profile_write' });
  assert.equal(await readFile(parent, 'utf8'), 'keep this file\n');
});
