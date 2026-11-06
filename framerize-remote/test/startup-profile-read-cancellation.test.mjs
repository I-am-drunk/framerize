import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import https from 'node:https';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import test from 'node:test';

const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));

async function bounded(promise, milliseconds, message) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolveUnused, reject) => {
      timer = setTimeout(() => reject(new Error(message)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

test('serve cancellation settles while its profile read is pending', { timeout: 5000 }, async context => {
  const directory = await fs.mkdtemp(join(tmpdir(), 'profile-read-cancel-'));
  const path = join(directory, 'profile.json');
  const originalRead = fs.readFile.bind(fs);
  const originalOpen = fs.open.bind(fs);
  const handles = [];
  const savedEnv = Object.entries(process.env).filter(([name]) => /^FRM_|^FRAMERIZE_REMOTE_HOME$/i.test(name));
  const signalCounts = ['SIGINT', 'SIGTERM'].map(name => process.listenerCount(name));
  let release = async () => {}, completion, cleanupPromise;
  let notifyEntered;
  const entered = new Promise(resolveEntered => { notifyEntered = resolveEntered; });
  const cleanup = () => cleanupPromise ??= (async () => {
    try {
      await release();
      if (completion) await bounded(completion, 1500, 'late startup did not settle');
      for (const handle of handles) await handle.close();
    } finally {
      context.mock.restoreAll();
      syncBuiltinESMExports();
      for (const [name, value] of savedEnv) process.env[name] = value;
      await fs.rm(directory, { recursive: true, force: true });
    }
  })();
  context.after(cleanup);
  for (const [name] of savedEnv) delete process.env[name];
  const profile = '{"remote":"http://127.0.0.1:9","host":"127.0.0.1","port":0}\n';
  await fs.writeFile(path, profile, { mode: 0o600 });
  let reads = 0, networkCalls = 0, receivedSignal = false;
  const hold = (read, options) => new Promise((resolveRead, reject) => {
    reads += 1;
    const signal = typeof options === 'object' ? options?.signal : undefined;
    receivedSignal = signal instanceof AbortSignal;
    const abort = () => reject(Object.assign(new Error('Synthetic read aborted'), { name: 'AbortError' }));
    signal?.addEventListener('abort', abort, { once: true });
    let released = false;
    release = async () => {
      if (released) return;
      released = true;
      signal?.removeEventListener('abort', abort);
      try { resolveRead(await read()); } catch (error) { reject(error); }
    };
    notifyEntered();
  });
  context.mock.method(fs, 'readFile', (name, options) => name === path
    ? hold(() => originalRead(name, options), options) : originalRead(name, options));
  context.mock.method(fs, 'open', async (...args) => {
    const handle = await originalOpen(...args);
    if (args[0] === path) {
      handles.push(handle);
      const read = handle.readFile.bind(handle);
      context.mock.method(handle, 'readFile', options => hold(() => read(options), options));
    }
    return handle;
  });
  for (const transport of [http, https]) context.mock.method(transport, 'request', () => {
    networkCalls += 1;
    throw new Error('No readiness request is allowed after startup cancellation');
  });
  syncBuiltinESMExports();
  const { run } = await import(pathToFileURL(join(packageRoot, 'dist/index.js')).href);
  const stdout = [], stderr = [];
  completion = run(['serve', '--config', path, '--json'], {
    stdout: text => stdout.push(text), stderr: text => stderr.push(text),
  }).then(code => ({ code }), error => ({ error: error.message }));
  await bounded(entered, 1500, 'profile read was not reached');
  assert.equal(process.listenerCount('SIGTERM'), signalCounts[1] + 1);
  process.emit('SIGTERM');
  const settledBeforeRelease = await bounded(completion.then(() => true), 1000,
    'startup still waits for the held read').catch(() => false);
  await release();
  const result = await bounded(completion, 1000, 'startup did not stop after the read');
  assert.equal(await originalRead(path, 'utf8'), profile);
  const afterCounts = ['SIGINT', 'SIGTERM'].map(name => process.listenerCount(name));
  await cleanup();
  context.diagnostic(JSON.stringify({ runtime: process.version, packageRoot, settledBeforeRelease,
    receivedSignal, reads, networkCalls, result, signal: 'emitted SIGTERM event', privateStateRemoved: true }));
  assert.deepEqual(result, { code: 0 });
  assert.deepEqual(stdout, ['{"stopped":true}\n']);
  assert.deepEqual(stderr, []);
  assert.deepEqual(afterCounts, signalCounts);
  assert.equal(networkCalls, 0);
  assert.equal(reads, 1);
  assert.equal(settledBeforeRelease, true, 'stopping serve must not wait for the held profile read');
});
