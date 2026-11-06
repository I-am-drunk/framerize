import test from 'node:test';
import assert from 'node:assert/strict';
import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/serviceCommand.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
stop();
const { executeServiceCommand } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function bounded(pending, milliseconds = 3000) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Owned command did not settle')), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

const running = 'console.log("ready"); setInterval(() => {}, 1000);';
const scenarios = [
  { name: 'a departed failed command retains its exit code',
    script: 'process.stdout.write("kept"); process.exit(7);', expected: { code: 7, stdout: 'kept' } },
  { name: 'a group containing only zombies retains the command failure',
    script: 'process.exit(7);', zombies: true, expected: { code: 7, stdout: '' } },
  { name: 'an exited overflowing command retains its bounded failure',
    script: 'process.stdout.write("x".repeat(4096));', expected: { code: 1, stdout: 'x'.repeat(64) } },
  { name: 'a live denied group remains a cleanup failure',
    script: running, abort: true, failure: true },
  { name: 'failed inspection cannot clear a denied signal',
    script: running, abort: true, failure: true, inspectionFailure: true },
  { name: 'other signalling errors remain failures without inspection',
    script: running, abort: true, failure: true, code: 'EACCES' },
  { name: 'a denied escalation accepts only a group that has stopped',
    script: running, abort: true, denied: 'SIGKILL', expected: { code: 1, stdout: 'ready\n' } },
];

for (const scenario of scenarios) test(scenario.name, {
  timeout: 6000, skip: process.platform === 'win32' && 'Requires POSIX process groups',
}, async t => {
  const originalSpawn = childProcess.spawn, originalExec = childProcess.execFile;
  const originalKill = process.kill;
  const controller = new AbortController();
  const ready = Promise.withResolvers();
  const signalError = Object.assign(new Error('Synthetic signal denial'), { code: scenario.code ?? 'EPERM' });
  let child, closed, pending, denials = 0, inspections = 0;
  childProcess.spawn = (...args) => {
    child = originalSpawn(...args);
    closed = new Promise(resolve => child.once('close', resolve));
    child.stdout.once('data', ready.resolve);
    return child;
  };
  process.kill = (pid, signal) => {
    if (pid === -child?.pid && signal === (scenario.denied ?? 'SIGTERM')) {
      denials++;
      throw signalError;
    }
    return originalKill(pid, signal);
  };
  childProcess.execFile = (command, ...args) => {
    if (command !== '/bin/ps') return originalExec(command, ...args);
    inspections++;
    if (scenario.inspectionFailure) {
      args.at(-1)(Object.assign(new Error('Synthetic inspection failure'), { code: 'EIO' }), '');
      return;
    }
    if (scenario.zombies) {
      args.at(-1)(null, `${child.pid} Z+\n`);
      return;
    }
    return originalExec(command, ...args);
  };
  syncBuiltinESMExports();
  t.after(async () => {
    childProcess.spawn = originalSpawn;
    childProcess.execFile = originalExec;
    process.kill = originalKill;
    syncBuiltinESMExports();
    child?.ref();
    if (child?.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    if (closed) await bounded(closed);
  });
  pending = executeServiceCommand(process.execPath, ['-e', scenario.script], 5000, 64,
    scenario.abort ? controller.signal : undefined);
  const outcome = pending.then(value => ({ value }), error => ({ error }));
  if (scenario.abort) {
    await bounded(ready.promise);
    controller.abort();
  }
  const result = await bounded(outcome);
  assert.equal(denials, 1);
  if (scenario.failure) {
    assert.equal(result.error?.code, 'service_cleanup_failed');
    assert.equal(result.error?.cause, signalError);
    assert.equal(child.exitCode, null);
    assert.equal(child.signalCode, null);
  } else {
    assert.ifError(result.error);
    assert.deepEqual(result.value, scenario.expected);
    await bounded(closed);
    assert.notEqual(child.exitCode ?? child.signalCode, null);
  }
  if (!scenario.failure) assert.ok(inspections > 0, 'Stopped group must be inspected before success');
  if (scenario.code === 'EACCES') assert.equal(inspections, 0);
});
