import assert from 'node:assert/strict';
import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
import { join } from 'node:path';
import test from 'node:test';
import { pathToFileURL } from 'node:url';

const moduleUrl = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ? pathToFileURL(join(process.env.FRAMERIZE_SERVER_PACKAGE_ROOT, 'dist/service-control.js'))
  : new URL('../dist/service-control.js', import.meta.url);
const { runStep, cancellableRunner } = await import(moduleUrl.href);

test('commands preserve stdout, failure status and literal arguments', { timeout: 5000 }, async () => {
  const value = 'héllo $(printf unexpected); * "quoted"';
  const args = ['-e', 'process.stdout.write(process.argv[1])', value];
  assert.deepEqual(await runStep(process.execPath, args), { code: 0, stdout: value });
  assert.deepEqual(await runStep(process.execPath, ['-e', 'process.stdout.write("kept"); process.exit(7)']),
    { code: 1, stdout: 'kept' });
  assert.deepEqual(await runStep(join(process.execPath, 'missing'), []), { code: 1, stdout: '' });
});

for (const stream of ['stdout', 'stderr']) test(`commands retain the ${stream} byte limit`, {
  timeout: 5000,
}, async () => {
  const result = await runStep(process.execPath, ['-e', `process.${stream}.write('x'.repeat(512 * 1024))`]);
  assert.equal(result.code, 1);
  assert.ok(Buffer.byteLength(result.stdout) <= 256 * 1024);
  if (stream === 'stderr') assert.equal(result.stdout, '');
});

test('an already cancelled command does not spawn', async t => {
  const original = childProcess.execFile;
  childProcess.execFile = () => assert.fail('cancelled command spawned');
  syncBuiltinESMExports();
  t.after(() => { childProcess.execFile = original; syncBuiltinESMExports(); });
  assert.deepEqual(await runStep(process.execPath, [], AbortSignal.abort()), { code: 1, stdout: '' });
});

test('a group stop failure survives cancellation mapping', {
  timeout: 5000, skip: process.platform === 'win32' && 'Requires POSIX process groups',
}, async t => {
  const originalExec = childProcess.execFile;
  const originalKill = process.kill;
  const controller = new AbortController();
  let child, closed;
  const ready = Promise.withResolvers();
  childProcess.execFile = (...args) => {
    child = originalExec(...args);
    closed = new Promise(resolve => child.once('close', resolve));
    child.stdout.once('data', ready.resolve);
    child.once('error', () => {});
    return child;
  };
  syncBuiltinESMExports();
  process.kill = (pid, signal) => {
    if (pid === -child?.pid) throw Object.assign(new Error('synthetic denial'), { code: 'EPERM' });
    return originalKill(pid, signal);
  };
  t.after(async () => {
    childProcess.execFile = originalExec;
    process.kill = originalKill;
    syncBuiltinESMExports();
    if (child?.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    await closed;
  });
  const pending = cancellableRunner(runStep, controller.signal)(process.execPath,
    ['-e', 'console.log("ready"); setInterval(() => {}, 1000)']);
  await ready.promise;
  const rejected = assert.rejects(pending, error => {
    assert.equal(error.cause?.code, 'EPERM');
    assert.match(error.message, /Some may still be running/);
    return true;
  });
  controller.abort(new Error('original cancellation'));
  await rejected;
});
