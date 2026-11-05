import assert from 'node:assert/strict';
import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
import { dirname, join } from 'node:path';
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
  assert.deepEqual(await runStep(join(dirname(process.execPath), 'framerize-missing-command'), []),
    { code: 1, stdout: '' });
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
  const original = childProcess.spawn;
  childProcess.spawn = () => assert.fail('cancelled command spawned');
  syncBuiltinESMExports();
  t.after(() => { childProcess.spawn = original; syncBuiltinESMExports(); });
  assert.deepEqual(await runStep(process.execPath, [], AbortSignal.abort()), { code: 1, stdout: '' });
});

for (const failure of ['signal', 'inspection']) test(`a group ${failure} failure survives cancellation mapping`, {
  timeout: 5000, skip: process.platform === 'win32' && 'Requires POSIX process groups',
}, async t => {
  const originalSpawn = childProcess.spawn;
  const originalExec = childProcess.execFile;
  const originalKill = process.kill;
  const controller = new AbortController();
  let child, closed;
  const ready = Promise.withResolvers();
  childProcess.spawn = (...args) => {
    child = originalSpawn(...args);
    closed = new Promise(resolve => child.once('close', resolve));
    child.stdout.once('data', ready.resolve);
    child.once('error', () => {});
    return child;
  };
  childProcess.execFile = (command, ...args) => {
    if (command === '/bin/ps' && failure === 'inspection') {
      args.at(-1)(Object.assign(new Error('synthetic inspection failure'), { code: 'EIO' }), '');
      return;
    }
    return originalExec(command, ...args);
  };
  syncBuiltinESMExports();
  process.kill = (pid, signal) => {
    if (failure === 'signal' && pid === -child?.pid) {
      throw Object.assign(new Error('synthetic denial'), { code: 'EPERM' });
    }
    return originalKill(pid, signal);
  };
  t.after(async () => {
    childProcess.spawn = originalSpawn;
    childProcess.execFile = originalExec;
    process.kill = originalKill;
    syncBuiltinESMExports();
    child?.ref();
    if (child?.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    await closed;
  });
  const pending = cancellableRunner(runStep, controller.signal)(process.execPath,
    ['-e', 'process.on("SIGTERM", () => {}); console.log("ready"); setInterval(() => {}, 1000)']);
  await ready.promise;
  const rejected = assert.rejects(pending, error => {
    assert.equal(error.cause?.code, failure === 'signal' ? 'EPERM' : 'EIO');
    assert.match(error.message, /Some may still be running/);
    return true;
  });
  controller.abort(new Error('original cancellation'));
  await rejected;
});

test('the command deadline force-stops a child that ignores termination', {
  timeout: 5000, skip: process.platform === 'win32' && 'Requires catchable POSIX signals',
}, async t => {
  const original = childProcess.spawn;
  const ready = Promise.withResolvers();
  let child, closed, settled = false;
  childProcess.spawn = (...args) => {
    child = original(...args);
    closed = new Promise(resolve => child.once('close', resolve));
    child.stdout.once('data', ready.resolve);
    return child;
  };
  syncBuiltinESMExports();
  t.mock.timers.enable({ apis: ['setTimeout'] });
  t.after(async () => {
    childProcess.spawn = original;
    syncBuiltinESMExports();
    t.mock.timers.reset();
    child?.ref();
    if (child?.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    await closed;
  });
  const pending = runStep(process.execPath,
    ['-e', 'process.on("SIGTERM", () => {}); console.log("ready"); setInterval(() => {}, 1000)'])
    .then(result => { settled = true; return result; });
  await ready.promise;
  t.mock.timers.tick(119_999);
  await Promise.resolve();
  assert.equal(settled, false, 'the command may run until its deadline');
  t.mock.timers.tick(1);
  t.mock.timers.tick(1000);
  t.mock.timers.reset();
  const result = await pending;
  assert.equal(result.code, 1);
  assert.equal(result.stdout, 'ready\n');
  assert.equal(child.signalCode, 'SIGKILL');
});
