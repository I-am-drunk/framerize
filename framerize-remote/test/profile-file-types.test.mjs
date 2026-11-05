import test from 'node:test';
import assert from 'node:assert/strict';
import { fork, execFileSync } from 'node:child_process';
import { constants } from 'node:fs';
import { mkdtemp, writeFile, mkdir, rm, symlink, lstat, open } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/configStore.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const source = compiled.outputFiles[0].text;
const { loadConfigFile } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const profile = { remote: 'http://127.0.0.1:8789', port: 0,
  serverToken: { kind: 'file', path: './synthetic-token' } };

async function within(pending, milliseconds, label) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(`${label} did not settle`)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function temporary(context) {
  const root = await mkdtemp(join(tmpdir(), 'frm-profile-file-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  return root;
}

async function assertNoFifoReader(file) {
  let handle;
  try {
    handle = await open(file, constants.O_WRONLY | constants.O_NONBLOCK);
    assert.fail('FIFO still has an open reader');
  } catch (error) { assert.equal(error.code, 'ENXIO'); }
  finally { await handle?.close(); }
}

test('profile loading rejects a FIFO without waiting for a writer', {
  timeout: 6000, skip: process.platform === 'win32',
}, async context => {
  const root = await mkdtemp(join(tmpdir(), 'frm-profile-fifo-'));
  const file = join(root, 'profile.json');
  let child;
  let closed;
  let didClose = false;
  context.after(async () => {
    try {
      if (child && !didClose) child.kill('SIGKILL');
      if (closed) await within(closed, 2000, 'captured child cleanup');
      if (child) {
        await assertNoFifoReader(file);
        context.diagnostic('cleanup: captured child closed; FIFO has no reader');
      }
    } finally { await rm(root, { recursive: true, force: true }); }
  });
  execFileSync('mkfifo', ['-m', '600', file], { timeout: 2000 });
  await writeFile(join(root, 'config-store.mjs'), source, { mode: 0o600 });
  await writeFile(join(root, 'reader.mjs'), `
import { loadConfigFile } from './config-store.mjs';
process.send({ event: 'reading' });
try {
  await loadConfigFile(process.argv[2]);
  process.send({ event: 'resolved' });
} catch (error) {
  process.send({ event: 'rejected', code: error.code });
}
process.disconnect();
`, { mode: 0o600 });
  let markReading;
  let markResult;
  const reading = new Promise(resolve => { markReading = resolve; });
  const result = new Promise(resolve => { markResult = resolve; });
  child = fork(join(root, 'reader.mjs'), [file], {
    execPath: process.execPath, stdio: ['ignore', 'pipe', 'pipe', 'ipc'],
    env: { HOME: root, XDG_CONFIG_HOME: root, TMPDIR: root },
  });
  child.stdout.resume();
  let stderr = '';
  child.stderr.on('data', chunk => { stderr = (stderr + chunk).slice(-4096); });
  child.on('message', message => {
    if (message.event === 'reading') markReading(message);
    else markResult(message);
  });
  child.once('error', error => {
    markReading({ event: 'spawn-error', code: error.code });
    markResult({ event: 'spawn-error', code: error.code });
  });
  closed = new Promise(resolve => child.once('close', (code, signal) => {
    didClose = true;
    markReading({ event: 'early-close', code, signal, stderr });
    markResult({ event: 'early-close', code, signal, stderr });
    resolve({ code, signal });
  }));
  assert.deepEqual(await within(reading, 2000, 'reader startup'), { event: 'reading' });
  const observed = await within(result, 1000, 'profile read without a FIFO writer');
  assert.deepEqual(observed, { event: 'rejected', code: 'profile_invalid' });
  assert.deepEqual(await within(closed, 2000, 'reader exit'), { code: 0, signal: null });
  assert.equal((await lstat(file)).isFIFO(), true);
});

test('regular profiles retain normalization and relative credential resolution', async context => {
  const root = await temporary(context);
  const file = join(root, 'profile.json');
  await writeFile(file, JSON.stringify(profile), { mode: 0o600 });
  const loaded = await loadConfigFile(file);
  assert.equal(loaded.remote, profile.remote);
  assert.equal(loaded.port, 0);
  assert.equal(loaded.host, '127.0.0.1');
  assert.deepEqual(loaded.serverToken, { kind: 'file', path: join(root, 'synthetic-token') });
});

test('missing, directory and malformed profiles keep their diagnostics', async context => {
  const root = await temporary(context);
  await assert.rejects(loadConfigFile(join(root, 'missing.json')), { code: 'profile_missing' });
  const directory = join(root, 'directory');
  await mkdir(directory);
  await assert.rejects(loadConfigFile(directory), { code: 'profile_invalid' });
  const malformed = join(root, 'malformed.json');
  await writeFile(malformed, '{', { mode: 0o600 });
  await assert.rejects(loadConfigFile(malformed), {
    code: 'profile_invalid', message: 'configuration file is not valid JSON',
  });
});

test('a symlink to a regular profile remains supported', {
  skip: process.platform === 'win32',
}, async context => {
  const root = await temporary(context);
  const file = join(root, 'profile.json');
  const link = join(root, 'linked.json');
  await writeFile(file, JSON.stringify(profile), { mode: 0o600 });
  await symlink(file, link);
  assert.deepEqual(await loadConfigFile(link), await loadConfigFile(file));
  assert.equal((await lstat(link)).isSymbolicLink(), true);
});
