import assert from 'node:assert/strict';
import test from 'node:test';
import { access, mkdir, mkdtemp, readFile, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';
import fs from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { createServer } from 'node:http';

const compiled = await build({ stdin: {
  contents: "export { withProfileLock } from './profileLock.ts'; export { runSetup } from './cmdSetup.ts'; export { setUp } from './cmdUp.ts'; export * from './diagnostics.ts';",
  resolveDir: fileURLToPath(new URL('../src/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false,
alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) } });
stop();
const { withProfileLock, runSetup, setUp, RemoteDiagnosticError, SetupRollbackError, commandFailure } = await import(
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
  await assert.rejects(setUp(args[1], {}, { configPath: join(parent, 'remote.json'),
    env: {}, service: {} }), { code: 'profile_invalid' });
  assert.equal(await readFile(parent, 'utf8'), 'keep this file\n');
});

test('an address prompt holds no lock and up rereads a completed setup', { timeout: 5000 }, async context => {
  const { root, profile, lock } = await fixture(context);
  const server = createServer((request, response) => {
    assert.equal(request.url, '/ready');
    response.setHeader('content-type', 'application/json');
    response.end(JSON.stringify({ apiVersion: 1, status: 'ready',
      capabilities: { responses: true, chatCompletions: true, streaming: true } }));
  });
  context.after(async () => {
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const remote = `http://127.0.0.1:${server.address().port}`;
  const entered = Promise.withResolvers(), answer = Promise.withResolvers();
  const io = { stdout() {}, stderr() {}, ask() { entered.resolve(); return answer.promise; } };
  const pending = setUp(undefined, io, { configPath: profile, env: {},
    service: { home: root, host: { platform: 'win32', homeDir: root },
      program: ['synthetic'], run() { assert.fail('No service command expected'); } } });
  void pending.catch(() => {});
  try {
    await Promise.race([entered.promise, pending.then(() => assert.fail('Expected an address prompt'))]);
    await missing(profile);
    await missing(lock);
    await runSetup([...args, '--port', '23456'], profile, {});
    answer.resolve(remote);
    const result = await pending;
    assert.equal(result.localApi, 'http://127.0.0.1:23456/v1');
    assert.equal(JSON.parse(await readFile(profile, 'utf8')).remote, remote);
    await missing(lock);
  } finally { answer.resolve(remote); await pending.catch(() => {}); }
});

for (const stage of ['close', 'remove']) for (const failed of [false, true]) {
  test(`${stage} failure preserves ${failed ? 'rollback diagnostics' : 'successful operation state'}`, async context => {
    const { profile, lock } = await fixture(context);
    const cleanupError = new Error('synthetic lock cleanup failure');
    const original = new SetupRollbackError(new RemoteDiagnosticError('aborted'), [new Error('rollback')]);
    const originalOpen = fs.open, originalRm = fs.rm;
    let closes = 0, removes = 0;
    context.after(() => { context.mock.restoreAll(); syncBuiltinESMExports(); });
    context.mock.method(fs, 'open', async (...args) => {
      const handle = await originalOpen(...args);
      if (args[0] !== lock) return handle;
      const close = handle.close.bind(handle);
      context.mock.method(handle, 'close', async () => {
        closes++;
        await close();
        if (stage === 'close') throw cleanupError;
      });
      return handle;
    });
    context.mock.method(fs, 'rm', async (path, ...args) => {
      if (path === lock) {
        removes++;
        if (stage === 'remove') throw cleanupError;
      }
      return originalRm(path, ...args);
    });
    syncBuiltinESMExports();
    const error = await withProfileLock(profile, async () => {
      if (failed) throw original;
      await writeFile(profile, 'operation completed');
    }).then(() => assert.fail('Cleanup must reject'), error => error);
    assert.equal(error.code, 'profile_lock_cleanup');
    assert.equal(error.original, failed ? original : undefined);
    assert.ok(error.cause.errors.includes(cleanupError));
    const diagnostic = JSON.parse(commandFailure(error, true));
    assert.equal(diagnostic.code, failed ? 'service_rollback_failed' : 'profile_lock_cleanup');
    if (failed) {
      assert.equal(diagnostic.setup.code, 'aborted');
      assert.equal(diagnostic.profileLock.code, 'profile_lock_cleanup');
    } else assert.equal(await readFile(profile, 'utf8'), 'operation completed');
    assert.equal(closes, 1);
    assert.equal(removes, 1);
    context.mock.restoreAll();
    syncBuiltinESMExports();
    if (stage === 'remove') {
      assert.equal(await readFile(lock, 'utf8'), `${process.pid}\n`);
      await originalRm(lock);
    }
    await missing(lock);
    await runSetup(args, profile, {});
    await missing(lock);
  });
}
