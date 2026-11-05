import test from 'node:test';
import assert from 'node:assert/strict';
import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
import { createConnection } from 'node:net';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/serviceControl.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
const { runStep } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function bounded(promise, fallback, milliseconds = 3000) {
  let timer;
  try { return await Promise.race([promise, new Promise(resolve => {
    timer = setTimeout(() => resolve(fallback), milliseconds);
  })]); } finally { clearTimeout(timer); }
}

async function listening(port) {
  const socket = createConnection({ host: '127.0.0.1', port });
  try { return await bounded(new Promise((resolve, reject) => {
    socket.once('connect', () => resolve(true));
    socket.once('error', error => error.code === 'ECONNREFUSED' ? resolve(false) : reject(error));
  }), 'probe timed out', 1000); } finally { socket.destroy(); }
}

const workerScript = `import { createServer } from 'node:net';
  process.on('SIGTERM', () => {});
  const server = createServer(socket => socket.end());
  server.listen(0, '127.0.0.1', () => {
    process.send({ pid: process.pid, port: server.address().port });
    process.disconnect();
  });`;

for (const [action, output] of [['cancel', 'inherit'], ['cancel', 'ignore'], ['fail', 'ignore'], ['success', 'ignore']]) {
  test(`${action} handles command descendants with ${output} output`, {
    timeout: 9000, skip: process.platform === 'win32' && 'Requires POSIX process groups',
  }, async t => {
    const originalSpawn = childProcess.spawn, originalExec = childProcess.execFile;
    const controller = new AbortController();
    const ready = Promise.withResolvers();
    let child, closed, worker, didClose = false;
    const capture = value => {
      assert.equal(child, undefined, 'only the owned launcher may be created');
      child = value;
      closed = new Promise(resolve => child.once('close', () => { didClose = true; resolve(true); }));
      let text = '';
      child.stdout.on('data', chunk => {
        text += chunk.toString('utf8');
        if (text.includes('\n')) ready.resolve(JSON.parse(text.trim()));
      });
      child.once('error', () => {});
      return child;
    };
    childProcess.spawn = (command, ...args) => {
      assert.equal(command, process.execPath);
      return capture(originalSpawn(command, ...args));
    };
    childProcess.execFile = (command, ...args) => {
      if (command === '/bin/ps') return originalExec(command, ...args);
      assert.equal(command, process.execPath);
      return capture(originalExec(command, ...args));
    };
    syncBuiltinESMExports();
    t.after(async () => {
      childProcess.spawn = originalSpawn;
      childProcess.execFile = originalExec;
      syncBuiltinESMExports();
      controller.abort();
      if (worker) {
        try { process.kill(worker.pid, 'SIGKILL'); }
        catch (error) { if (error.code !== 'ESRCH') throw error; }
      }
      if (child?.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
      if (closed) assert.equal(await bounded(closed, false), true, 'owned child must close');
      if (worker) {
        const deadline = Date.now() + 1000;
        while (await listening(worker.port) && Date.now() < deadline) {
          await new Promise(resolve => setTimeout(resolve, 10));
        }
        assert.equal(await listening(worker.port), false, 'owned listener must close');
      }
      t.diagnostic('cleanup: owned child closed and worker port refused connections');
    });
    const script = `import { spawn } from 'node:child_process';
      process.on('SIGTERM', () => process.exit(0));
      const worker = spawn(process.execPath, ['--input-type=module', '-e', ${JSON.stringify(workerScript)}],
        { stdio: ['ignore', '${output}', '${output}', 'ipc'] });
      worker.once('message', value => process.stdout.write(JSON.stringify(value) + '\\n', () => {
        if ('${action}' === 'fail') process.exit(7);
        if ('${action}' === 'success') worker.unref();
      }));`;
    const pending = runStep(process.execPath, ['--input-type=module', '-e', script], controller.signal)
      .then(result => ({ result, didClose }));
    pending.catch(() => {});
    worker = await bounded(ready.promise, undefined);
    assert.ok(worker && Number.isSafeInteger(worker.pid) && worker.pid > 1 && worker.pid !== process.pid);
    assert.ok(Number.isInteger(worker.port) && worker.port > 0);
    if (action !== 'fail') assert.equal(await listening(worker.port), true);
    if (action === 'cancel') controller.abort();
    const outcome = await bounded(pending, undefined);
    const workerListening = await listening(worker.port);
    t.diagnostic(JSON.stringify({ action, output, outcome, workerListening }));
    assert.ok(outcome, 'command must settle even if descendants inherit its output');
    assert.equal(outcome.result.code, action === 'success' ? 0 : action === 'fail' ? 7 : 1);
    assert.equal(outcome.didClose, true, 'command pipes must close before rollback');
    assert.equal(workerListening, action === 'success', 'failed or cancelled commands must stop descendants');
  });
}
