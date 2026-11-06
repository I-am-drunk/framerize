import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, fork } from 'node:child_process';
import { constants } from 'node:fs';
import { lstat, mkdir, mkdtemp, open, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/settings.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const moduleSource = compiled.outputFiles[0].text;
const { readSettings } = await import(`data:text/javascript;base64,${Buffer.from(moduleSource).toString('base64')}`);
const saved = { version: 1, config: { backend: 'http', host: '127.0.0.1', port: 0 },
  environment: { SYNTHETIC_LABEL: 'private 🦊 fixture' }, access: { tailscalePort: 9443 } };

async function within(pending, milliseconds, label) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(`${label} did not settle`)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function temporary(context) {
  const root = await mkdtemp(join(tmpdir(), 'server-settings-type-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  return root;
}

async function noFifoReader(file) {
  let handle;
  try {
    handle = await open(file, constants.O_WRONLY | constants.O_NONBLOCK);
    assert.fail('settings FIFO still has a reader');
  } catch (error) { assert.equal(error.code, 'ENXIO'); }
  finally { await handle?.close(); }
}

test('Server settings reject a FIFO without waiting for a writer', {
  timeout: 6000, skip: process.platform === 'win32',
}, async context => {
  const root = await mkdtemp(join(tmpdir(), 'server-settings-fifo-'));
  const file = join(root, 'server.json');
  let child;
  let exited = false;
  const closed = Promise.withResolvers();
  context.after(async () => {
    try {
      if (child && !exited) child.kill('SIGKILL');
      if (child) {
        await within(closed.promise, 2000, 'captured reader cleanup');
        await noFifoReader(file);
        context.diagnostic('cleanup: captured child closed; Server settings FIFO has no reader');
      }
    } finally { await rm(root, { recursive: true, force: true }); }
  });
  execFileSync('mkfifo', ['-m', '600', file], { timeout: 2000 });
  await writeFile(join(root, 'settings.mjs'), moduleSource, { mode: 0o600 });
  await writeFile(join(root, 'reader.mjs'), `
import { readSettings } from './settings.mjs';
process.send({ event: 'reading' });
try {
  await readSettings(process.argv[2]);
  process.send({ event: 'resolved' });
} catch (error) {
  process.send({ event: 'rejected', name: error.name, message: error.message });
}
process.disconnect();
`, { mode: 0o600 });
  const entered = Promise.withResolvers();
  const result = Promise.withResolvers();
  child = fork(join(root, 'reader.mjs'), [file], {
    execPath: process.execPath, stdio: ['ignore', 'pipe', 'pipe', 'ipc'],
    env: { HOME: root, XDG_CONFIG_HOME: root, TMPDIR: root },
  });
  child.stdout.resume();
  let stderr = '';
  child.stderr.on('data', chunk => { stderr = (stderr + chunk).slice(-4096); });
  child.on('message', message => {
    if (message.event === 'reading') entered.resolve(message);
    else result.resolve(message);
  });
  child.once('error', error => {
    entered.resolve({ event: 'error', code: error.code });
    result.resolve({ event: 'error', code: error.code });
  });
  child.once('close', (code, signal) => {
    exited = true;
    entered.resolve({ event: 'early-close', code, signal, stderr });
    result.resolve({ event: 'early-close', code, signal, stderr });
    closed.resolve({ code, signal });
  });
  assert.deepEqual(await within(entered.promise, 2000, 'reader startup'), { event: 'reading' });
  assert.deepEqual(await within(result.promise, 1000, 'settings read without a FIFO writer'), {
    event: 'rejected', name: 'ConfigurationError', message: 'Server settings are unavailable or invalid',
  });
  assert.deepEqual(await within(closed.promise, 2000, 'reader exit'), { code: 0, signal: null });
  assert.equal((await lstat(file)).isFIFO(), true);
});

test('regular Server settings retain configuration, environment and access metadata', async context => {
  const root = await temporary(context);
  const file = join(root, 'server.json');
  await writeFile(file, JSON.stringify(saved), { mode: 0o600 });
  assert.deepEqual(await readSettings(file), saved);
});

test('regular-file Server settings symlinks remain supported', {
  skip: process.platform === 'win32',
}, async context => {
  const root = await temporary(context);
  const file = join(root, 'server.json');
  const link = join(root, 'linked.json');
  await writeFile(file, JSON.stringify(saved), { mode: 0o600 });
  await symlink(file, link);
  assert.deepEqual(await readSettings(link), saved);
  assert.equal((await lstat(link)).isSymbolicLink(), true);
});

test('unreadable and malformed Server settings keep their diagnostics', async context => {
  const root = await temporary(context);
  const directory = join(root, 'directory');
  const file = join(root, 'server.json');
  await mkdir(directory);
  for (const path of [directory, join(root, 'missing.json')]) {
    await assert.rejects(readSettings(path), { message: 'Server settings are unavailable or invalid' });
  }
  await writeFile(file, '{', { mode: 0o600 });
  await assert.rejects(readSettings(file), { message: 'Server settings are unavailable or invalid' });
  await writeFile(file, JSON.stringify({ version: 2, config: {}, environment: {} }));
  await assert.rejects(readSettings(file), { message: 'Invalid Server settings' });
});
