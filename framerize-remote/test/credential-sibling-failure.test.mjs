import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { join, resolve } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import test from 'node:test';
import { build } from 'esbuild';

const sourceRoot = resolve(process.env.FRAMERIZE_REMOTE_SOURCE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const compiled = await build({
  entryPoints: [join(sourceRoot, 'src/resolveInstance.ts')],
  alias: { '@framerize/core': join(sourceRoot, '../framerize-core/src/index.ts') },
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
});
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const inputs = await Promise.all(Object.keys(compiled.metafile.inputs).sort()
  .map(async path => ({ path, sha256: hash(await fs.readFile(path)) })));
const bundle = compiled.outputFiles[0].contents;
const { resolveInstance } = await import(`data:text/javascript;base64,${Buffer.from(bundle).toString('base64')}`);
console.log(JSON.stringify({ node: process.version, platform: process.platform,
  inputs, bundleSha256: hash(bundle) }));

async function within(pending, label, milliseconds = 1500) {
  let timer;
  try { return await Promise.race([pending, new Promise((resolveUnused, reject) => {
    timer = setTimeout(() => reject(new Error(`${label} did not settle`)), milliseconds);
  })]); } finally { clearTimeout(timer); }
}

for (const failedRole of ['server', 'local']) {
  test(`${failedRole} credential failure cancels the pending sibling read`, { timeout: 6000 }, async context => {
    const root = await fs.mkdtemp(join(tmpdir(), 'credential-sibling-'));
    const profile = join(root, 'profile.json');
    const failedPath = join(root, 'invalid-token');
    const siblingPath = join(root, 'pending-token');
    const entered = Promise.withResolvers();
    const release = Promise.withResolvers();
    const readSettled = Promise.withResolvers();
    const closed = Promise.withResolvers();
    const controller = new AbortController();
    const handles = [];
    const originalOpen = fs.open.bind(fs);
    let operation, siblingHandle, reading = false, cleanupPromise;
    const cleanup = () => cleanupPromise ??= (async () => {
      release.resolve();
      controller.abort(new Error('Owned fixture cleanup'));
      try {
        if (operation) await within(operation, 'resolution cleanup');
        if (reading) await within(readSettled.promise, 'read cleanup');
        await within(Promise.all(handles.map(handle => handle.close())), 'handle cleanup');
      } finally {
        context.mock.restoreAll();
        syncBuiltinESMExports();
        await fs.rm(root, { recursive: true, force: true });
      }
      await assert.rejects(fs.stat(root), { code: 'ENOENT' });
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    })();
    context.after(cleanup);
    const profileBytes = JSON.stringify({ remote: 'http://127.0.0.1:9', port: 0,
      serverToken: { kind: 'file', path: failedRole === 'server' ? failedPath : siblingPath },
      localToken: { kind: 'file', path: failedRole === 'local' ? failedPath : siblingPath } });
    await fs.writeFile(profile, profileBytes, { mode: 0o600 });
    await fs.writeFile(failedPath, 'not a bearer token', { mode: 0o600 });
    await fs.writeFile(siblingPath, 'synthetic-valid-token', { mode: 0o600 });
    context.mock.method(fs, 'open', async (...args) => {
      const handle = await originalOpen(...args);
      if (args[0] !== failedPath && args[0] !== siblingPath) return handle;
      handles.push(handle);
      const read = handle.read.bind(handle);
      if (args[0] === siblingPath) {
        siblingHandle = handle;
        const close = handle.close.bind(handle);
        context.mock.method(handle, 'close', async () => {
          await close();
          closed.resolve();
        });
        context.mock.method(handle, 'read', async (...readArgs) => {
          reading = true;
          entered.resolve();
          try { await release.promise; return await read(...readArgs); }
          finally { readSettled.resolve(); }
        });
      } else context.mock.method(handle, 'read', async (...readArgs) => {
        await entered.promise;
        return read(...readArgs);
      });
      return handle;
    });
    syncBuiltinESMExports();
    operation = resolveInstance(profile, {}, undefined, false, controller.signal)
      .then(value => ({ value }), error => ({ error }));
    await within(entered.promise, 'sibling read entry');
    const result = await within(operation, 'credential failure');
    const closedBeforeRelease = await within(closed.promise, 'sibling close', 750)
      .then(() => true, () => false);
    const listenersAfterFailure = getEventListeners(controller.signal, 'abort').length;
    const externallyAborted = controller.signal.aborted;
    const fdAfterFailure = siblingHandle.fd;
    assert.equal(await fs.readFile(profile, 'utf8'), profileBytes);
    assert.equal(await fs.readFile(failedPath, 'utf8'), 'not a bearer token');
    assert.equal(await fs.readFile(siblingPath, 'utf8'), 'synthetic-valid-token');
    await cleanup();
    context.diagnostic(JSON.stringify({ failedRole, code: result.error?.code,
      closedBeforeRelease, listenersAfterFailure, externallyAborted,
      siblingWasOpenAfterFailure: fdAfterFailure >= 0, stateRemoved: true }));
    assert.equal(result.error?.code, `${failedRole}_credential`);
    assert.equal(externallyAborted, false);
    assert.equal(closedBeforeRelease, true, 'Failed resolution must cancel its sibling read');
    assert.equal(fdAfterFailure, -1, 'Sibling credential handle must close before fixture release');
    assert.equal(listenersAfterFailure, 0);
  });
}

test('both healthy credential reads still resolve without cancelling the caller', { timeout: 3000 }, async context => {
  const root = await fs.mkdtemp(join(tmpdir(), 'credential-sibling-control-'));
  const controller = new AbortController();
  let resolved;
  context.after(async () => {
    await resolved?.backend.close();
    await fs.rm(root, { recursive: true, force: true });
    await assert.rejects(fs.stat(root), { code: 'ENOENT' });
    context.diagnostic('healthy control: backend closed and private state removed');
  });
  const serverPath = join(root, 'server-token');
  const localPath = join(root, 'local-token');
  await fs.writeFile(serverPath, 'synthetic-server-token', { mode: 0o600 });
  await fs.writeFile(localPath, 'synthetic-local-token', { mode: 0o600 });
  const profile = join(root, 'profile.json');
  await fs.writeFile(profile, JSON.stringify({ remote: 'http://127.0.0.1:9', port: 0,
    serverToken: { kind: 'file', path: serverPath },
    localToken: { kind: 'file', path: localPath } }), { mode: 0o600 });
  resolved = await resolveInstance(profile, {}, undefined, false, controller.signal);
  assert.equal(resolved.localToken, 'synthetic-local-token');
  assert.equal(resolved.config.serverToken.path, serverPath);
  assert.equal(controller.signal.aborted, false);
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
});
