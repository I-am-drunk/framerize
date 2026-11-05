import test from 'node:test';
import assert from 'node:assert/strict';
import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const source = name => fileURLToPath(new URL(`../src/${name}.ts`, import.meta.url));
const compiled = await build({ stdin: { contents: ['serviceControl', 'serviceCommand', 'diagnostics']
  .map(name => `export * from ${JSON.stringify(source(name))};`).join('\n'),
  resolveDir: fileURLToPath(new URL('.', import.meta.url)) },
bundle: true, platform: 'node', format: 'esm', write: false });
const service = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

test('service commands preserve literal arguments, stdout and numeric failures', { timeout: 5000 }, async () => {
  const text = 'héllo $(printf unexpected); * "quoted"';
  assert.deepEqual(await service.runStep(process.execPath, ['-e', 'process.stdout.write(process.argv[1])', text]),
    { code: 0, stdout: text });
  assert.deepEqual(await service.runStep(process.execPath, ['-e', 'process.stdout.write("kept"); process.exit(7)']),
    { code: 7, stdout: 'kept' });
  assert.deepEqual(await service.runStep(join(dirname(process.execPath), 'frm-missing-command'), []),
    { code: 1, stdout: '' });
});

for (const limit of [64 * 1024, 256 * 1024]) for (const stream of ['stdout', 'stderr']) {
  test(`service commands bound ${stream} at ${limit} bytes`, { timeout: 5000 }, async () => {
    const result = await service.executeServiceCommand(process.execPath,
      ['-e', `process.${stream}.write('x'.repeat(${limit + 65536}))`], 5000, limit);
    assert.equal(result.code, 1);
    assert.ok(Buffer.byteLength(result.stdout) <= limit);
    if (stream === 'stderr') assert.equal(result.stdout, '');
  });
}

test('pre-cancelled service commands create no child', async t => {
  const original = childProcess.spawn;
  childProcess.spawn = () => assert.fail('cancelled command spawned');
  syncBuiltinESMExports();
  t.after(() => { childProcess.spawn = original; syncBuiltinESMExports(); });
  assert.deepEqual(await service.runStep(process.execPath, [], AbortSignal.abort()), { code: 1, stdout: '' });
});

for (const failure of ['signal', 'inspection']) test(`service command ${failure} failure has a cleanup diagnostic`, {
  timeout: 5000, skip: process.platform === 'win32' && 'Requires POSIX process groups',
}, async t => {
  const originalSpawn = childProcess.spawn, originalExec = childProcess.execFile, originalKill = process.kill;
  const controller = new AbortController(), ready = Promise.withResolvers();
  let child, closed;
  childProcess.spawn = (...args) => {
    child = originalSpawn(...args);
    closed = new Promise(resolve => child.once('close', resolve));
    child.stdout.once('data', ready.resolve);
    child.once('error', () => {});
    return child;
  };
  childProcess.execFile = (command, ...args) => {
    if (command === '/bin/ps' && failure === 'inspection') {
      args.at(-1)(Object.assign(new Error('synthetic-private-failure'), { code: 'EIO' }), '');
      return;
    }
    return originalExec(command, ...args);
  };
  process.kill = (pid, signal) => {
    if (failure === 'signal' && pid === -child?.pid) {
      throw Object.assign(new Error('synthetic-private-denial'), { code: 'EPERM' });
    }
    return originalKill(pid, signal);
  };
  syncBuiltinESMExports();
  t.after(async () => {
    childProcess.spawn = originalSpawn;
    childProcess.execFile = originalExec;
    process.kill = originalKill;
    syncBuiltinESMExports();
    child?.ref();
    if (child?.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    await closed;
  });
  const pending = service.runStep(process.execPath,
    ['-e', 'process.on("SIGTERM", () => {}); console.log("ready"); setInterval(() => {}, 1000)'], controller.signal);
  await ready.promise;
  const rejected = assert.rejects(pending, error => {
    assert.equal(error.code, 'service_cleanup_failed');
    assert.equal(error.cause?.code, failure === 'signal' ? 'EPERM' : 'EIO');
    assert.equal(JSON.parse(service.commandFailure(error, true)).error, 'service_cleanup_failed');
    assert.match(service.commandFailure(error, false), /Some may still be running/);
    assert.doesNotMatch(service.commandFailure(error, false), /synthetic-private/);
    return true;
  });
  controller.abort();
  await rejected;
});

for (const timeout of [5000, 120000]) test(`service command deadline remains ${timeout} ms`, {
  timeout: 5000, skip: process.platform === 'win32' && 'Requires catchable POSIX signals',
}, async t => {
  const original = childProcess.spawn, ready = Promise.withResolvers();
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
  const args = ['-e', 'process.on("SIGTERM", () => {}); console.log("ready"); setInterval(() => {}, 1000)'];
  const command = timeout === 120000 ? service.runStep(process.execPath, args)
    : service.executeServiceCommand(process.execPath, args, timeout, 64 * 1024);
  const pending = command.then(result => { settled = true; return result; });
  await ready.promise;
  t.mock.timers.tick(timeout - 1);
  await Promise.resolve();
  assert.equal(settled, false);
  t.mock.timers.tick(1);
  t.mock.timers.tick(1000);
  t.mock.timers.reset();
  assert.deepEqual(await pending, { code: 1, stdout: 'ready\n' });
  assert.equal(child.signalCode, 'SIGKILL');
});
