import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { EventEmitter } from 'node:events';
import { PassThrough } from 'node:stream';
import { mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setImmediate as nextTurn } from 'node:timers/promises';
import { createHash } from 'node:crypto';
import { build } from 'esbuild';

async function scenario(root) {
  const compiled = await build({
    entryPoints: [fileURLToPath(new URL('../src/browser-login.ts', import.meta.url))],
    bundle: true, platform: 'node', format: 'esm', write: false,
  });
  const { loginWithBrowser } = await import(
    `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
  const controller = new AbortController();
  const commands = new PassThrough(), replies = new PassThrough();
  const child = Object.assign(new EventEmitter(), {
    pid: 424242, stdio: [null, null, null, commands, replies],
  });
  const methods = [], unhandled = [];
  let alive = false, spawns = 0, confirmations = 0;
  const onUnhandled = error => unhandled.push(error?.code ?? 'unknown');
  process.on('unhandledRejection', onUnhandled);
  commands.on('data', bytes => {
    const message = JSON.parse(bytes.toString().replace(/\0$/, ''));
    methods.push(message.method);
    assert.equal(message.method, 'Browser.close');
    alive = false;
    child.emit('exit', 0, null);
  });
  let result;
  try {
    await loginWithBrowser({ interactive: true, platform: 'linux', env: { DISPLAY: ':99' },
      browser: process.execPath, temporaryDirectory: root, timeoutMs: 1500,
      signal: controller.signal, notice() {},
      spawn() { spawns++; alive = true; return child; },
      kill(pid, signal) {
        assert.equal(pid, -child.pid);
        assert.equal(signal, 0);
        if (!alive) throw Object.assign(new Error('Simulated stopped group'), { code: 'ESRCH' });
      },
      async confirm(signal) {
        confirmations++;
        assert.equal(signal.aborted, false);
        controller.abort();
        throw new Error('Synthetic prompt rejection after cancellation');
      },
    });
    result = 'unexpected-success';
  } catch (error) { result = error.code ?? 'unknown'; }
  await nextTurn();
  await nextTurn();
  process.off('unhandledRejection', onUnhandled);
  console.log(JSON.stringify({ result, unhandled, confirmations, spawns, methods,
    alive, streamsDestroyed: commands.destroyed && replies.destroyed,
    remaining: await readdir(root),
    bundleSha256: createHash('sha256').update(compiled.outputFiles[0].contents).digest('hex') }));
}

async function bounded(promise) {
  let timer;
  try { return await Promise.race([promise, new Promise((resolveUnused, reject) => {
    timer = setTimeout(() => reject(new Error('Owned worker did not close')), 2500);
  })]); } finally { clearTimeout(timer); }
}

if (process.argv[2] === 'scenario') {
  await scenario(process.argv[3]);
} else {
  const { default: test } = await import('node:test');
  test('browser cancellation consumes a prompt rejection at the abort boundary', { timeout: 6000 }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'browser-prompt-race-'));
    let worker, closed;
    context.after(async () => {
      if (worker && worker.exitCode === null && worker.signalCode === null) worker.kill('SIGKILL');
      try { if (closed) await bounded(closed); }
      finally { await rm(root, { recursive: true, force: true }); }
    });
    worker = spawn(process.execPath, [fileURLToPath(import.meta.url), 'scenario', root], {
      env: { PATH: dirname(process.execPath), HOME: root, TMPDIR: root },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let stdout = '', stderr = '';
    worker.stdout.setEncoding('utf8').on('data', text => { stdout += text; });
    worker.stderr.setEncoding('utf8').on('data', text => { stderr += text; });
    closed = new Promise((resolve, reject) => {
      worker.once('error', reject);
      worker.once('close', (code, signal) => resolve({ code, signal }));
    });
    assert.deepEqual(await bounded(closed), { code: 0, signal: null }, stderr);
    assert.equal(stderr, '');
    const report = JSON.parse(stdout);
    context.diagnostic(JSON.stringify(report));
    assert.equal(report.result, 'cancelled');
    assert.equal(report.confirmations, 1);
    assert.equal(report.spawns, 1);
    assert.deepEqual(report.methods, ['Browser.close']);
    assert.equal(report.alive, false);
    assert.equal(report.streamsDestroyed, true);
    assert.deepEqual(report.remaining, []);
    assert.deepEqual(report.unhandled, [], 'Cancellation must consume the rejected prompt promise');
  });
}
