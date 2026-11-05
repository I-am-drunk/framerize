import test from 'node:test';
import assert from 'node:assert/strict';
import childProcess from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { get } from 'node:http';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const source = fileURLToPath(new URL('../src/serviceControl.ts', import.meta.url));
const compiled = await build({ entryPoints: [source], bundle: true,
  platform: 'node', format: 'esm', write: false });
const bytes = compiled.outputFiles[0].contents;
const bundleSha256 = createHash('sha256').update(bytes).digest('hex');
const { runStep } = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

async function bounded(pending, message) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(message)), 4000);
    })]);
  } finally { clearTimeout(timer); }
}

function readiness(port) {
  return new Promise((resolve, reject) => {
    const outgoing = get({ host: '127.0.0.1', port, path: '/ready', agent: false,
      signal: AbortSignal.timeout(1000) }, response => {
      let body = '';
      response.setEncoding('utf8');
      response.on('data', chunk => { body += chunk; });
      response.once('error', reject);
      response.once('end', () => resolve({ status: response.statusCode, body }));
    });
    outgoing.once('error', reject);
  });
}

for (const behavior of ['delayed', 'ignore']) {
  test(`runStep cancellation drains a child with ${behavior} SIGTERM handling`,
    { timeout: 5000, skip: process.platform === 'win32' ? 'Requires catchable POSIX signals' : false },
    async context => {
      const root = await mkdtemp(join(tmpdir(), 'remote-service-cancel-'));
      const script = join(root, 'service.mjs');
      const args = [script, behavior];
      const controller = new AbortController();
      const originalExecFile = childProcess.execFile;
      const ready = Promise.withResolvers();
      let child, closed, port, settled, exitObserved = false, closeObserved = false;
      let invocations = 0;
      const running = () => child && child.exitCode === null && child.signalCode === null;
      context.after(async () => {
        childProcess.execFile = originalExecFile;
        syncBuiltinESMExports();
        try {
          controller.abort();
          if (running()) child.kill('SIGKILL');
          if (closed) await bounded(closed, 'Captured child cleanup timed out');
          if (settled) await bounded(settled.catch(() => {}), 'runStep cleanup timed out');
          if (port) {
            await assert.rejects(readiness(port), { code: 'ECONNREFUSED' });
            context.diagnostic(JSON.stringify({ cleanupPid: child.pid, port, portRefused: true }));
          }
        } finally { await rm(root, { recursive: true, force: true }); }
      });
      await writeFile(script, `
import { createServer } from 'node:http';
const server = createServer((request, response) => {
  response.writeHead(request.url === '/ready' ? 200 : 404, { 'content-type': 'application/json' });
  response.end(JSON.stringify({ pid: process.pid }));
});
let stopping = false;
process.on('SIGTERM', () => {
  if (process.argv[2] === 'ignore' || stopping) return;
  stopping = true;
  setTimeout(() => server.close(() => process.exit(0)), 300);
});
server.listen(0, '127.0.0.1', () => {
  process.stdout.write(JSON.stringify({ pid: process.pid, port: server.address().port }) + '\\n');
});
`, { mode: 0o600 });
      childProcess.execFile = (command, suppliedArgs, options, callback) => {
        assert.equal(command, process.execPath, 'Only the synthetic Node child may run');
        assert.deepEqual(suppliedArgs, args);
        assert.equal(++invocations, 1, 'Only one child may be created');
        child = originalExecFile(command, suppliedArgs, options, callback);
        child.once('exit', () => { exitObserved = true; });
        closed = new Promise(resolve => child.once('close', (code, signal) => {
          closeObserved = true;
          resolve({ code, signal });
        }));
        child.once('error', ready.reject);
        let output = '';
        child.stdout.setEncoding('utf8').on('data', chunk => {
          output += chunk;
          if (!output.includes('\n')) return;
          try { ready.resolve(JSON.parse(output.split('\n')[0])); }
          catch (error) { ready.reject(error); }
        });
        return child;
      };
      syncBuiltinESMExports();
      settled = runStep(process.execPath, args, controller.signal).then(result => ({
        result, exitObserved, closeObserved, running: running(),
        exitCode: child.exitCode, signalCode: child.signalCode,
      }));
      settled.catch(() => {});
      const address = await bounded(Promise.race([ready.promise, settled.then(() => {
        throw new Error('runStep settled before the synthetic child was ready');
      })]), 'Synthetic child did not become ready');
      assert.equal(address.pid, child.pid);
      port = address.port;
      assert.ok(Number.isInteger(port) && port > 0 && port <= 65535);
      const before = await readiness(port);
      assert.equal(before.status, 200);
      assert.deepEqual(JSON.parse(before.body), { pid: child.pid });
      controller.abort();
      const observed = await bounded(settled, 'Cancelled service command did not settle');
      const listener = await readiness(port).then(() => 'open', error => error.code);
      context.diagnostic(JSON.stringify({ behavior, pid: child.pid, port,
        bundleSha256, bundleBytes: bytes.length, ...observed, listener }));
      assert.equal(typeof observed.result.code, 'number');
      assert.notEqual(observed.result.code, 0, 'Aborted commands must report failure');
      assert.equal(typeof observed.result.stdout, 'string');
      assert.deepEqual({ exited: observed.exitObserved, closed: observed.closeObserved,
        running: observed.running, listener }, {
        exited: true, closed: true, running: false, listener: 'ECONNREFUSED',
      }, 'runStep must not release setup rollback while its service command is alive');
    });
}
