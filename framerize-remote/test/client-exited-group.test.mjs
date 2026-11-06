import test from 'node:test';
import assert from 'node:assert/strict';
import childProcess from 'node:child_process';
import fs from 'node:fs';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/process.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
stop();
const { nodeRunner } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function bounded(pending, milliseconds = 3000) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Owned client command did not settle')), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

const scenarios = [
  { name: 'a departed failed client retains its exit code', expected: 7 },
  { name: 'a client group containing only zombies retains its exit code', states: 'Z+', expected: 7 },
  { name: 'a client terminated by a signal retains its failure', signalExit: true, expected: 1 },
  { name: 'a reported live client group remains a cleanup failure', states: 'S', failure: true },
  { name: 'failed group inspection cannot clear a client signal denial', inspectionFailure: true, failure: true },
  { name: 'other client signal errors remain failures', code: 'EACCES', failure: true },
  { name: 'mixed client signal errors remain failures', mixed: true, failure: true },
  { name: 'missing client groups retain ordinary command failures', code: 'ESRCH', expected: 7 },
];

for (const scenario of scenarios) test(scenario.name, {
  timeout: 5000, skip: process.platform === 'win32' && 'Requires POSIX process groups',
}, async t => {
  const originalSpawn = childProcess.spawn, originalExec = childProcess.execFile;
  const originalKill = process.kill, originalOpen = fs.openSync;
  const controller = new AbortController();
  const failures = [], signals = [];
  let child, closed, inspections = 0;
  fs.openSync = (path, ...args) => {
    if (path === '/dev/tty') throw new Error('Synthetic nonterminal command');
    return originalOpen(path, ...args);
  };
  process.kill = (pid, signal) => {
    if (pid !== -child?.pid) return originalKill(pid, signal);
    signals.push(signal);
    const code = scenario.mixed && signal === 'SIGKILL' ? 'EACCES' : scenario.code ?? 'EPERM';
    const error = Object.assign(new Error('Synthetic client signal denial'), { code });
    failures.push(error);
    throw error;
  };
  childProcess.execFile = (command, ...args) => {
    assert.equal(command, '/bin/ps', 'Only the bounded group query is permitted');
    inspections++;
    if (scenario.inspectionFailure) {
      args.at(-1)(Object.assign(new Error('Synthetic inspection failure'), { code: 'EIO' }), '');
    } else if (scenario.states) {
      args.at(-1)(null, `${child.pid} ${scenario.states}\n`);
    } else return originalExec(command, ...args);
  };
  syncBuiltinESMExports();
  t.after(async () => {
    process.kill = originalKill;
    fs.openSync = originalOpen;
    childProcess.execFile = originalExec;
    syncBuiltinESMExports();
    if (child?.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    if (closed) await bounded(closed);
  });
  const runner = nodeRunner(process.platform, (...args) => {
    child = originalSpawn(...args);
    closed = new Promise(resolve => child.once('close', resolve));
    return child;
  });
  const script = scenario.signalExit ? 'process.kill(process.pid, "SIGTERM")' : 'process.exit(7)';
  const outcome = await bounded(runner.run({ command: process.execPath, args: ['-e', script],
    display: 'synthetic failed client' }, {}, controller.signal)
    .then(value => ({ value }), error => ({ error })));
  await bounded(closed);
  assert.equal(child.exitCode, scenario.signalExit ? null : 7);
  assert.equal(child.signalCode, scenario.signalExit ? 'SIGTERM' : null);
  assert.deepEqual(getEventListeners(controller.signal, 'abort'), []);
  assert.deepEqual(signals, ['SIGTERM', 'SIGKILL']);
  if (scenario.failure) {
    assert.equal(outcome.error?.code, 'client_cleanup_failed');
    assert.deepEqual(outcome.error.cause.errors, failures, 'Retain the original signal failures');
  } else {
    assert.ifError(outcome.error);
    assert.equal(outcome.value, scenario.expected);
    if (scenario.code !== 'ESRCH') assert.ok(inspections > 0, 'Confirm the group has stopped');
  }
  if (scenario.code === 'EACCES' || scenario.mixed || scenario.code === 'ESRCH') assert.equal(inspections, 0);
  t.diagnostic(JSON.stringify({ inspections, exitCode: child.exitCode, signal: child.signalCode,
    code: outcome.error?.code ?? outcome.value, childClosed: true, abortListeners: 0 }));
});
