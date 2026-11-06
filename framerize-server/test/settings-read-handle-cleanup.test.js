import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/settings.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
stop();
const { readSettings } = await import(`data:text/javascript;base64,${
  Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const savedText = ' { "version": 1, "config": {}, "environment": {} }\n';

async function within(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolveUnused, reject) => {
      timer = setTimeout(() => reject(new Error('Settings fixture did not settle')), 1000);
    })]);
  } finally { clearTimeout(timer); }
}

for (const stage of ['open', 'stat', 'readFile', 'close']) {
  for (const outcome of ['success', 'failure']) {
    test(`settings cancel during ${stage}, late ${outcome} closes once`, { timeout: 5000 }, async context => {
      const root = await fs.mkdtemp(join(tmpdir(), 'settings-handle-cancel-'));
      const path = join(root, 'server.json');
      const originalOpen = fs.open;
      const originalRead = fs.readFile;
      const entered = Promise.withResolvers();
      const release = Promise.withResolvers();
      const stageFinished = Promise.withResolvers();
      const closed = Promise.withResolvers();
      const controller = new AbortController();
      const reason = new Error('cancel selected settings operation');
      const calls = { open: 0, stat: 0, readFile: 0, close: 0 };
      let handle, pending, enteredStage = false;
      async function hold() {
        enteredStage = true;
        entered.resolve({ entered: true });
        try {
          await release.promise;
          if (outcome === 'failure') throw Object.assign(new Error('late file failure'), { code: 'EIO' });
        } finally { stageFinished.resolve(); }
      }
      context.after(async () => {
        release.resolve();
        try {
          if (pending) await within(pending);
          if (enteredStage) await within(stageFinished.promise);
          if (handle) await within(closed.promise);
        } finally {
          context.mock.restoreAll();
          syncBuiltinESMExports();
          if (handle && handle.fd !== -1) await handle.close();
          await fs.rm(root, { recursive: true, force: true });
          await assert.rejects(fs.stat(root), { code: 'ENOENT' });
        }
      });
      await fs.writeFile(path, savedText, { mode: 0o600 });
      context.mock.method(fs, 'open', async function (file, ...options) {
        assert.equal(String(file), path);
        calls.open++;
        if (stage === 'open' && outcome === 'failure') await hold();
        handle = await originalOpen.call(this, file, ...options);
        const close = handle.close.bind(handle);
        context.mock.method(handle, 'close', async () => {
          calls.close++;
          await close();
          closed.resolve();
          if (stage === 'close') await hold();
        });
        for (const method of ['stat', 'readFile']) {
          const original = handle[method].bind(handle);
          context.mock.method(handle, method, async (...args) => {
            calls[method]++;
            const value = await original(...args);
            if (stage === method) await hold();
            return value;
          });
        }
        if (stage === 'open') await hold();
        return handle;
      });
      syncBuiltinESMExports();
      pending = readSettings(path, controller.signal).then(value => ({ value }), error => ({ error }));
      assert.equal((await within(Promise.race([entered.promise, pending]))).entered, true);
      controller.abort(reason);
      assert.equal((await within(pending)).error, reason);
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
      assert.equal(calls.close, stage === 'open' ? 0 : 1);
      if (stage !== 'open') await within(closed.promise);
      release.resolve();
      await within(stageFinished.promise);
      if (handle) await within(closed.promise);
      await new Promise(resolve => setImmediate(resolve));
      assert.deepEqual(calls, {
        open: 1,
        stat: stage === 'open' ? 0 : 1,
        readFile: ['readFile', 'close'].includes(stage) ? 1 : 0,
        close: stage === 'open' && outcome === 'failure' ? 0 : 1,
      });
      if (handle) assert.equal(handle.fd, -1);
      assert.equal(await originalRead(path, 'utf8'), savedText);
      assert.deepEqual(await fs.readdir(root), ['server.json']);
      context.diagnostic(JSON.stringify({ stage, outcome, calls, cancellationReasonPreserved: true }));
    });
  }
}
