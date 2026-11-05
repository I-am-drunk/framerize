import assert from 'node:assert/strict';
import childProcess from 'node:child_process';
import { once } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { createConnection } from 'node:net';
import { join } from 'node:path';
import test from 'node:test';
import { pathToFileURL } from 'node:url';

const moduleUrl = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ? pathToFileURL(join(process.env.FRAMERIZE_SERVER_PACKAGE_ROOT, 'dist/service-control.js'))
  : new URL('../dist/service-control.js', import.meta.url);
const { runStep } = await import(moduleUrl.href);

async function bounded(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Child did not settle')), 2500);
    })]);
  } finally { clearTimeout(timer); }
}

for (const delayed of [false, true]) test(`cancelled command closes with ${delayed ? 'delayed' : 'ignored'} SIGTERM`, {
  timeout: 6000, skip: process.platform === 'win32' && 'Requires catchable POSIX signals',
}, async t => {
  const original = childProcess.execFile;
  const controller = new AbortController();
  let child, closed, port, didClose = false;
  const ready = Promise.withResolvers();
  childProcess.execFile = (...args) => {
    child = original(...args);
    closed = new Promise(resolve => child.once('close', () => { didClose = true; resolve(); }));
    let output = '';
    child.stdout.setEncoding('utf8').on('data', chunk => {
      output += chunk;
      if (output.includes('\n')) ready.resolve(Number(output.trim()));
    });
    child.once('error', () => {});
    return child;
  };
  syncBuiltinESMExports();
  t.after(async () => {
    childProcess.execFile = original;
    syncBuiltinESMExports();
    controller.abort();
    if (child?.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    if (closed) await bounded(closed);
    if (port) {
      const socket = createConnection({ host: '127.0.0.1', port });
      try { assert.equal((await bounded(once(socket, 'error')))[0].code, 'ECONNREFUSED'); }
      finally { socket.destroy(); }
    }
  });
  const script = `import { createServer } from 'node:net';
    const server = createServer(socket => socket.end());
    process.on('SIGTERM', () => { ${delayed ? 'setTimeout(() => server.close(() => process.exit(0)), 100);' : ''} });
    server.listen(0, '127.0.0.1', () => console.log(server.address().port));`;
  const pending = runStep(process.execPath, ['--input-type=module', '-e', script], controller.signal)
    .then(result => ({ result, didClose }));
  port = await bounded(ready.promise);
  assert.ok(Number.isInteger(port) && port > 0);
  controller.abort();
  const outcome = await bounded(pending);
  assert.notEqual(outcome.result.code, 0);
  assert.equal(outcome.didClose, true, 'rollback must not race the command');
  assert.ok(child.exitCode !== null || child.signalCode !== null);
});
