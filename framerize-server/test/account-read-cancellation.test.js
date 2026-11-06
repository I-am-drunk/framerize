import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import https from 'node:https';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/account-backend.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { createAccountBackend } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

async function within(pending, label, milliseconds = 1500) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(`${label} did not settle`)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

for (const stage of ['open', 'read']) {
  test(`account startup cancels while metadata ${stage} is pending`, { timeout: 6000 }, async context => {
    const root = await fs.mkdtemp(join(tmpdir(), 'framerize-account-read-'));
    const path = join(root, 'accounts.json');
    const bytes = JSON.stringify([{ id: 'synthetic', projectId: 'synthetic-project',
      session: { env: 'SYNTHETIC_SESSION' } }]);
    const held = deferred();
    const release = deferred();
    const controller = new AbortController();
    const originalOpen = fs.open;
    const handles = [];
    let operation;
    let holdCount = 0;
    let sessionReads = 0;
    let minterCalls = 0;
    let networkCalls = 0;
    context.after(async () => {
      controller.abort();
      release.resolve();
      try {
        const result = operation && await within(operation, 'operation cleanup');
        await result?.backend?.close();
        for (const captured of handles) {
          if (!captured.closed) await within(captured.handle.close(), 'handle cleanup');
        }
      } finally {
        context.mock.restoreAll();
        syncBuiltinESMExports();
        await fs.rm(root, { recursive: true, force: true });
      }
      assert(handles.every(captured => captured.closed));
      await assert.rejects(fs.stat(root), { code: 'ENOENT' });
      context.diagnostic(`cleanup: ${handles.length} owned file handles closed; private state removed`);
    });
    await fs.writeFile(path, bytes, { mode: 0o600 });
    const forbidNetwork = () => { networkCalls++; throw new Error('No network is permitted'); };
    context.mock.method(http, 'request', forbidNetwork);
    context.mock.method(https, 'request', forbidNetwork);
    context.mock.method(globalThis, 'fetch', forbidNetwork);
    context.mock.method(fs, 'open', async (...args) => {
      const handle = await originalOpen(...args);
      if (args[0] !== path) return handle;
      const captured = { handle, closed: false };
      handles.push(captured);
      const close = handle.close.bind(handle);
      context.mock.method(handle, 'close', async () => {
        await close();
        captured.closed = true;
      });
      const pause = async () => {
        holdCount++;
        held.resolve();
        await release.promise;
      };
      if (stage === 'open') await pause();
      else {
        const read = handle.read.bind(handle);
        context.mock.method(handle, 'read', async (...readArgs) => {
          const result = await read(...readArgs);
          if (holdCount === 0) await pause();
          return result;
        });
      }
      return handle;
    });
    syncBuiltinESMExports();
    const env = Object.defineProperty({}, 'SYNTHETIC_SESSION', { get() {
      sessionReads++;
      throw new Error('Session must not be read after cancellation');
    } });
    operation = createAccountBackend({ accountsFile: path, env,
      origin: 'http://127.0.0.1:1', signal: controller.signal,
      createMinter() { minterCalls++; throw new Error('Minter must not be constructed'); },
    }).then(backend => ({ status: 'fulfilled', backend }), error => ({ status: 'rejected', error }));
    await within(Promise.race([held.promise, operation.then(() => {
      throw new Error('Startup settled before the metadata operation was held');
    })]), 'metadata hold');
    controller.abort(new Error('synthetic account startup cancellation'));
    const early = await within(operation, 'cancellation before metadata release', 500)
      .catch(error => ({ status: 'pending', error }));
    context.diagnostic(JSON.stringify({ stage, beforeRelease: early.status,
      code: early.error?.code, closedHandles: handles.filter(handle => handle.closed).length }));
    release.resolve();
    const result = await within(operation, 'startup after metadata release');
    assert.equal(result.status, 'rejected');
    assert.equal(result.error.code, 'request_cancelled');
    assert.equal(result.error.message, 'Framer account startup cancelled');
    assert.equal(await fs.readFile(path, 'utf8'), bytes);
    assert.equal(holdCount, 1);
    assert.equal(sessionReads, 0);
    assert.equal(minterCalls, 0);
    assert.equal(networkCalls, 0);
    context.diagnostic('After release: cancelled; profile unchanged; no session, minter, or network access');
    assert.equal(early.status, 'rejected', 'Startup must cancel without waiting for metadata I/O');
    assert.equal(early.error.code, 'request_cancelled');
  });
}
