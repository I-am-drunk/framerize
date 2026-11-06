import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs/promises';
import { readFileSync } from 'node:fs';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build, stop } from 'esbuild';

const root = process.env.FRAMERIZE_REMOTE_SOURCE_ROOT ?? fileURLToPath(new URL('../', import.meta.url));
const compiled = await build({ entryPoints: [join(root, 'src/cmdSetup.ts')],
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
stop();
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
console.log(JSON.stringify({ synthetic: true, bundle: hash(compiled.outputFiles[0].contents),
  inputs: Object.keys(compiled.metafile.inputs).map(path => ({ path, sha256: hash(readFileSync(path)) })) }));
const { runSetup } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function observe(pending, milliseconds) {
  let timer;
  try { return await Promise.race([pending, new Promise(resolve => {
    timer = setTimeout(() => resolve({ timedOut: true }), milliseconds);
  })]); } finally { clearTimeout(timer); }
}

for (const lateFailure of [false, true]) {
  test(`setup cancels a pending profile-lock path before its late ${lateFailure ? 'failure' : 'success'}`,
    { timeout: 5000 }, async context => {
      const directory = await fs.mkdtemp(join(tmpdir(), 'profile-lock-cancel-'));
      const controller = new AbortController();
      const reason = new Error('Synthetic setup cancellation');
      const openCalls = [], handles = [];
      let release = () => {}, pending, held, cleaned = false;
      const cleanup = async () => {
        if (cleaned) return;
        controller.abort(reason);
        release();
        if (pending) assert.equal((await observe(pending, 1500)).timedOut, undefined);
        if (held) await held.catch(() => {});
        for (const handle of handles) if (handle.fd >= 0) await handle.close();
        context.mock.restoreAll();
        syncBuiltinESMExports();
        await fs.rm(directory, { recursive: true, force: true });
        cleaned = true;
      };
      context.after(cleanup);
      const path = join(directory, 'profile.json');
      const original = '{"remote":"http://127.0.0.1:8789","port":8788}\n';
      await fs.writeFile(path, original, { mode: 0o600 });
      const originalRealpath = fs.realpath.bind(fs), originalOpen = fs.open.bind(fs);
      let enteredResolve;
      const entered = new Promise(resolve => { enteredResolve = resolve; });
      context.mock.method(fs, 'realpath', (requested, ...args) => {
        if (String(requested) !== resolve(directory)) return originalRealpath(requested, ...args);
        held = originalRealpath(requested, ...args).then(async value => {
          await new Promise(resolve => { release = resolve; enteredResolve(true); });
          if (lateFailure) throw Object.assign(new Error('Synthetic late path failure'), { code: 'EIO' });
          return value;
        });
        return held;
      });
      context.mock.method(fs, 'open', async (requested, ...args) => {
        openCalls.push({ path: String(requested), afterAbort: controller.signal.aborted });
        const handle = await originalOpen(requested, ...args);
        handles.push(handle);
        return handle;
      });
      syncBuiltinESMExports();
      pending = runSetup(['--remote', 'http://127.0.0.1:9876'], path, {}, undefined,
        controller.signal).then(value => ({ value }), error => ({ error }));
      assert.equal(await observe(entered, 1000), true, 'The canonical parent lookup must be pending');
      controller.abort(reason);
      const beforeRelease = await observe(pending, 300);
      const bytesBeforeRelease = await fs.readFile(path, 'utf8');
      release();
      const afterRelease = await observe(pending, 1000);
      await held.catch(() => {});
      const bytesAfterRelease = await fs.readFile(path, 'utf8');
      const remaining = await fs.readdir(directory);
      const liveHandles = handles.filter(handle => handle.fd >= 0).length;
      const abortListeners = getEventListeners(controller.signal, 'abort').length;
      await cleanup();
      const opensAfterAbort = openCalls.filter(entry => entry.afterAbort).length;
      context.diagnostic(JSON.stringify({ synthetic: true, lateFailure,
        rejectedBeforeRelease: beforeRelease.error === reason,
        rejectedAfterRelease: afterRelease.error === reason,
        rejectionCode: afterRelease.error?.code, opensAfterAbort,
        liveHandles, abortListeners, privateStateRemoved: cleaned }));
      await assert.rejects(fs.access(directory), { code: 'ENOENT' });
      assert.equal(bytesBeforeRelease, original);
      assert.equal(bytesAfterRelease, original);
      assert.deepEqual(remaining, ['profile.json']);
      assert.equal(liveHandles, 0);
      assert.equal(abortListeners, 0);
      assert.equal(opensAfterAbort, 0, 'No profile lock may be opened after cancellation');
      assert.equal(afterRelease.error, reason, 'A late filesystem failure must not replace cancellation');
      assert.equal(beforeRelease.error, reason,
        'Setup must reject cancellation before the held path lookup is released');
    });
}
