import assert from 'node:assert/strict';
import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
import { createConnection } from 'node:net';
import { join } from 'node:path';
import test from 'node:test';
import { pathToFileURL } from 'node:url';

const moduleUrl = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ? pathToFileURL(join(process.env.FRAMERIZE_SERVER_PACKAGE_ROOT, 'dist/service-control.js'))
  : new URL('../dist/service-control.js', import.meta.url);
const { runStep } = await import(moduleUrl.href);

async function bounded(promise, fallback, milliseconds = 2500) {
  let timer;
  try {
    return await Promise.race([promise, new Promise(resolve => {
      timer = setTimeout(() => resolve(fallback), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function listening(port) {
  const socket = createConnection({ host: '127.0.0.1', port });
  try {
    return await bounded(new Promise((resolve, reject) => {
      socket.once('connect', () => resolve(true));
      socket.once('error', error => error.code === 'ECONNREFUSED' ? resolve(false) : reject(error));
    }), 'probe timed out', 1000);
  } finally { socket.destroy(); }
}

const workerScript = `import { createServer } from 'node:net';
  process.on('SIGTERM', () => {});
  const server = createServer(socket => socket.end());
  server.listen(0, '127.0.0.1', () => {
    process.send({ pid: process.pid, port: server.address().port });
    process.disconnect();
  });`;

for (const output of ['inherit', 'ignore']) test(`cancellation stops descendants with ${output} output`, {
  timeout: 8500, skip: process.platform === 'win32' && 'Requires POSIX process groups and signals',
}, async t => {
  const original = childProcess.execFile;
  const controller = new AbortController();
  const ready = Promise.withResolvers();
  let child, closed, worker, didClose = false, didExit = false;
  childProcess.execFile = (...args) => {
    child = original(...args);
    closed = new Promise(resolve => child.once('close', () => { didClose = true; resolve(true); }));
    child.once('exit', () => { didExit = true; });
    let text = '';
    child.stdout.setEncoding('utf8').on('data', chunk => {
      text += chunk;
      if (text.includes('\n')) ready.resolve(JSON.parse(text.trim()));
    });
    child.once('error', () => {});
    return child;
  };
  syncBuiltinESMExports();
  t.after(async () => {
    childProcess.execFile = original;
    syncBuiltinESMExports();
    controller.abort();
    if (worker) {
      try { process.kill(worker.pid, 'SIGKILL'); }
      catch (error) { if (error.code !== 'ESRCH') throw error; }
    }
    if (child?.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    if (closed) assert.equal(await bounded(closed, false), true, 'fixture child must close');
    if (worker) assert.equal(await listening(worker.port), false, 'fixture listener must close');
    t.diagnostic('cleanup: captured child closed and owned port refused connections');
  });
  const script = `import { spawn } from 'node:child_process';
    process.on('SIGTERM', () => process.exit(0));
    const worker = spawn(process.execPath, ['--input-type=module', '-e', ${JSON.stringify(workerScript)}],
      { stdio: ['ignore', '${output}', '${output}', 'ipc'] });
    worker.once('message', value => console.log(JSON.stringify(value)));`;
  const pending = runStep(process.execPath, ['--input-type=module', '-e', script], controller.signal)
    .then(result => ({ result, didClose }));
  worker = await bounded(ready.promise, undefined);
  assert.ok(worker && Number.isSafeInteger(worker.pid) && worker.pid > 1 && worker.pid !== process.pid);
  assert.ok(Number.isInteger(worker.port) && worker.port > 0);
  assert.equal(await listening(worker.port), true);
  controller.abort();
  const outcome = await bounded(pending, undefined);
  const workerListening = await listening(worker.port);
  t.diagnostic(JSON.stringify({ returned: !!outcome, didExit, didClose, workerListening, result: outcome?.result }));
  assert.ok(outcome, 'cancellation must settle before rollback even when a descendant inherits stdout');
  assert.equal(outcome.result.code, 1);
  assert.equal(outcome.didClose, true, 'command pipes must close before rollback');
  assert.equal(workerListening, false, 'cancellation must stop descendants before rollback');
});
