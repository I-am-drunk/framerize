import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import https from 'node:https';
import { createHash } from 'node:crypto';
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
const bundleHash = createHash('sha256').update(compiled.outputFiles[0].contents).digest('hex');

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

async function within(pending, milliseconds = 1500) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error('Fixture wait did not settle')), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

for (const stage of ['open', 'read']) {
  test(`account startup cancels during session-file ${stage}`, { timeout: 6000 }, async context => {
    context.diagnostic(`source bundle SHA256 ${bundleHash}`);
    const root = await fs.mkdtemp(join(tmpdir(), 'account-session-cancel-'));
    const accountsPath = join(root, 'accounts.json');
    const sessionPath = join(root, 'session.txt');
    const session = 'synthetic-session\n';
    const accounts = JSON.stringify([{ id: 'synthetic', projectId: 'synthetic-project',
      session: { file: sessionPath } }]);
    const controller = new AbortController();
    const held = deferred();
    const release = deferred();
    const originalOpen = fs.open;
    const handles = [];
    let operation;
    let holdCount = 0;
    let minterCalls = 0;
    let networkCalls = 0;
    context.after(async () => {
      controller.abort();
      release.resolve();
      try {
        const result = operation && await within(operation);
        await result?.backend?.close();
      } finally {
        for (const captured of handles) {
          if (!captured.closed) await within(captured.handle.close());
        }
        context.mock.restoreAll();
        syncBuiltinESMExports();
        await fs.rm(root, { recursive: true, force: true });
      }
      assert(handles.every(captured => captured.closed));
      await assert.rejects(fs.stat(root), { code: 'ENOENT' });
      context.diagnostic(`cleanup: ${handles.length} session handle closed; private state removed`);
    });
    await fs.writeFile(accountsPath, accounts, { mode: 0o600 });
    await fs.writeFile(sessionPath, session, { mode: 0o600 });
    const forbidNetwork = () => { networkCalls++; throw new Error('No network is permitted'); };
    context.mock.method(http, 'request', forbidNetwork);
    context.mock.method(https, 'request', forbidNetwork);
    context.mock.method(globalThis, 'fetch', forbidNetwork);
    context.mock.method(fs, 'open', async (...args) => {
      const handle = await originalOpen(...args);
      if (args[0] !== sessionPath) return handle;
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
    operation = createAccountBackend({ accountsFile: accountsPath, env: {},
      origin: 'http://127.0.0.1:1', signal: controller.signal,
      createMinter() { minterCalls++; throw new Error('Minter must not be constructed'); },
    }).then(backend => ({ status: 'fulfilled', backend }), error => ({ status: 'rejected', error }));
    await within(Promise.race([held.promise, operation.then(() => {
      throw new Error('Startup settled before the session-file operation was held');
    })]));
    controller.abort(new Error('synthetic session-file cancellation'));
    const early = await within(operation, 500).catch(error => ({ status: 'pending', error }));
    context.diagnostic(JSON.stringify({ stage, beforeRelease: early.status,
      code: early.error?.code, closedHandles: handles.filter(handle => handle.closed).length }));
    release.resolve();
    const result = await within(operation);
    assert.equal(result.status, 'rejected');
    assert.equal(result.error.code, 'request_cancelled');
    assert.equal(result.error.message, 'Framer account startup cancelled');
    assert.equal(await fs.readFile(accountsPath, 'utf8'), accounts);
    assert.equal(await fs.readFile(sessionPath, 'utf8'), session);
    assert.equal(holdCount, 1);
    assert.equal(minterCalls, 0);
    assert.equal(networkCalls, 0);
    context.diagnostic('After release: cancelled; both files unchanged; no minter or network access');
    assert.equal(early.status, 'rejected', 'Startup must cancel while session-file I/O is pending');
    assert.equal(early.error.code, 'request_cancelled');
  });
}
