import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, lstat, open, rm } from 'node:fs/promises';
import { constants } from 'node:fs';
import { execFileSync, spawn } from 'node:child_process';
import { dirname, join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build } from 'esbuild';

const compiled = await build({
  stdin: { contents: "export { serviceStatus } from './src/service-control.ts'; export { selectServiceManager } from './src/service-units.ts';",
    resolveDir: fileURLToPath(new URL('../', import.meta.url)), loader: 'ts' },
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const bytes = compiled.outputFiles[0].contents;
const sourceDigest = createHash('sha256').update(bytes).digest('hex');
const moduleUrl = `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`;
const api = await import(moduleUrl);
const planFor = root => ({ home: join(root, 'state'), homeDir: root,
  program: [process.execPath, join(root, 'synthetic-launcher.js'), 'serve'],
  environmentPath: dirname(process.execPath), platform: 'linux', uid: process.getuid?.() ?? 1000,
  xdgConfigHome: join(root, 'config') });

const childSource = `
  import fs from 'node:fs/promises';
  import { syncBuiltinESMExports } from 'node:module';
  const [moduleUrl, serialized, path] = process.argv.slice(1);
  const original = fs.readFile;
  fs.readFile = (...args) => {
    if (args[0] === path) process.send({ event: 'read-entered' });
    return original(...args);
  };
  syncBuiltinESMExports();
  const { serviceStatus } = await import(moduleUrl);
  try {
    await serviceStatus(JSON.parse(serialized), async () => {
      process.send({ event: 'unexpected-service-command' });
      throw new Error('Service command must not run');
    });
    process.send({ event: 'result', returned: true }, () => process.disconnect());
  } catch (error) {
    process.send({ event: 'result', error: error.message }, () => process.disconnect());
  }
`;

async function bounded(promise, milliseconds, message) {
  let timer;
  try { return await Promise.race([promise, new Promise((resolve, reject) => {
    timer = setTimeout(() => reject(new Error(message)), milliseconds);
  })]); } finally { clearTimeout(timer); }
}

test('Server service status rejects a FIFO without waiting for a writer',
  { timeout: 6000, skip: process.platform === 'win32' }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'frm-server-unit-file-'));
    let child;
    let closed;
    let exit;
    const cleanup = async () => {
      try {
        if (child && !exit) { child.kill('SIGKILL'); await bounded(closed, 1500, 'child did not close'); }
      } finally { await rm(root, { recursive: true, force: true }); }
    };
    context.after(cleanup);
    const plan = planFor(root);
    const manager = api.selectServiceManager(plan);
    await mkdir(dirname(manager.path), { recursive: true });
    execFileSync('mkfifo', ['-m', '600', manager.path], { timeout: 1000 });
    const inode = (await lstat(manager.path)).ino;
    const entered = Promise.withResolvers();
    const messages = [];
    let stderr = '';
    child = spawn(process.execPath, ['--input-type=module', '-e', childSource,
      moduleUrl, JSON.stringify(plan), manager.path], {
      env: { HOME: root, TMPDIR: root, PATH: dirname(process.execPath) },
      stdio: ['ignore', 'ignore', 'pipe', 'ipc'],
    });
    closed = new Promise((resolve, reject) => {
      child.once('error', reject);
      child.once('close', (code, signal) => { exit = { code, signal }; resolve(exit); });
    });
    child.stderr.on('data', chunk => { stderr = (stderr + chunk).slice(0, 4096); });
    child.on('message', message => {
      messages.push(message);
      if (message.event === 'read-entered') entered.resolve();
    });
    await bounded(Promise.race([entered.promise, closed]), 2000, 'no read entry or child close');
    let settledBeforeCleanup = false;
    try { await bounded(closed, 750, 'read remained pending'); settledBeforeCleanup = true; }
    catch (error) { if (error.message !== 'read remained pending') throw error; }
    const observedExit = exit;
    if (!exit) { child.kill('SIGKILL'); await bounded(closed, 1500, 'child did not close'); }
    const after = await lstat(manager.path);
    let writerError;
    try {
      const handle = await open(manager.path, constants.O_WRONLY | constants.O_NONBLOCK);
      await handle.close();
    } catch (error) { writerError = error.code; }
    await cleanup();
    context.diagnostic(JSON.stringify({ node: process.version, sourceDigest,
      settledBeforeCleanup, observedExit, cleanupExit: exit, writerError,
      messages, privateStateRemoved: true }));
    assert.equal(stderr, '');
    assert.equal(after.isFIFO(), true);
    assert.equal(after.ino, inode);
    assert.equal(writerError, 'ENXIO', 'the owned FIFO reader must be gone');
    assert.equal(messages.some(message => message.event === 'unexpected-service-command'), false);
    assert.equal(settledBeforeCleanup, true, 'a FIFO unit must settle without fixture cleanup');
    assert.deepEqual(observedExit, { code: 0, signal: null });
    assert.equal(typeof messages.find(message => message.event === 'result')?.error, 'string');
  });

test('regular and missing Server units retain status behavior', { timeout: 2000 }, async context => {
  const root = await mkdtemp(join(tmpdir(), 'frm-server-unit-control-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  const plan = planFor(root);
  const manager = api.selectServiceManager(plan);
  const calls = [];
  const run = async (command, args) => {
    calls.push([command, ...args]);
    assert.equal(command, 'systemctl');
    assert.ok(args.includes('show') || args.includes('is-enabled'));
    return { code: 0, stdout: args.includes('show')
      ? 'ActiveState=inactive\nSubState=dead\nMainPID=0\n' : 'enabled\n' };
  };
  await mkdir(dirname(manager.path), { recursive: true });
  await writeFile(manager.path, manager.contents, { mode: 0o600 });
  const regular = await api.serviceStatus(plan, run);
  const contents = await readFile(manager.path, 'utf8');
  await rm(manager.path);
  const missing = await api.serviceStatus(plan, run);
  await rm(root, { recursive: true, force: true });
  assert.equal(contents, manager.contents);
  assert.equal(regular.installed, true);
  assert.equal(regular.current, true);
  assert.equal(regular.running, false);
  assert.equal(regular.otherHome, false);
  assert.equal(missing.installed, false);
  assert.equal(missing.running, false);
  assert.equal(missing.current, false);
  assert.ok(calls.length > 0);
  context.diagnostic('status-only synthetic commands; private state removed');
});
