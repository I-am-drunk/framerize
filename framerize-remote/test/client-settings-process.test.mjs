import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL(
  '../src/compatibility/settings.ts', import.meta.url))],
bundle: true, platform: 'node', format: 'esm', write: false });
const bundle = compiled.outputFiles[0].contents;
const moduleUrl = `data:text/javascript;base64,${Buffer.from(bundle).toString('base64')}`;
const bundleSha256 = createHash('sha256').update(bundle).digest('hex');
const { writeClaudeSettings, undoClaudeSettings, SettingsError } = await import(moduleUrl);

async function within(promise, label) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(label)), 1500);
    })]);
  } finally { clearTimeout(timer); }
}

const workerSource = `
  import fs from 'node:fs/promises';
  import { syncBuiltinESMExports } from 'node:module';
  let release;
  process.on('message', async message => {
    if (message.type === 'release') { release?.(); return; }
    if (message.type !== 'start') return;
    const rename = fs.rename;
    let held = false;
    fs.rename = async (from, to) => {
      if (to === message.settings && !held) {
        held = true;
        await new Promise(resolve => {
          release = resolve;
          process.send({ type: 'holding', pid: process.pid });
        });
      }
      return rename(from, to);
    };
    syncBuiltinESMExports();
    try {
      const { writeClaudeSettings } = await import(message.moduleUrl);
      await writeClaudeSettings(message.settings, message.state, message.target);
      process.send({ type: 'saved' }, () => process.disconnect());
    } catch (error) {
      process.send({ type: 'failed', code: error.code ?? error.name }, () => process.disconnect());
    }
  });
`;

test('a separate process cannot replace another writer\'s settings transaction',
  { timeout: 6000 }, async context => {
    const root = await fs.realpath(await fs.mkdtemp(join(tmpdir(), 'frm-settings-process-')));
    let child;
    let closed;
    let exit;
    const cleanup = async () => {
      try {
        if (child && !exit) {
          child.kill('SIGKILL');
          await within(closed, 'owned settings worker did not close');
        }
      } finally { await fs.rm(root, { recursive: true, force: true }); }
    };
    context.after(cleanup);
    const settings = join(root, 'settings.json');
    const state = join(root, 'state');
    const record = join(state, 'claude-settings.json');
    const original = { model: 'synthetic-model', env: { KEEP: 'synthetic-value' } };
    await fs.writeFile(settings, JSON.stringify(original), { mode: 0o600 });
    const holding = Promise.withResolvers();
    const messages = [];
    let stderr = '';
    child = spawn(process.execPath, ['--input-type=module', '-e', workerSource], {
      cwd: root, env: { HOME: root, TMPDIR: root, PATH: dirname(process.execPath) },
      stdio: ['ignore', 'ignore', 'pipe', 'ipc'],
    });
    closed = new Promise((resolve, reject) => {
      child.once('error', reject);
      child.once('close', (code, signal) => { exit = { code, signal }; resolve(exit); });
    });
    void closed.catch(() => {});
    child.stderr.on('data', chunk => { stderr = (stderr + chunk).slice(0, 4096); });
    child.on('message', message => {
      messages.push(message);
      if (message.type === 'holding') holding.resolve(message);
    });
    child.send({ type: 'start', moduleUrl, settings, state,
      target: { origin: 'http://127.0.0.1:18788', localToken: 'synthetic-worker-token' } });
    const ready = await within(Promise.race([holding.promise, closed.then(() => {
      throw new Error('worker closed before holding its transaction');
    })]), 'worker did not enter settings rename');
    assert.equal(ready.pid, child.pid);
    assert.notEqual(ready.pid, process.pid);
    const recordBefore = await fs.readFile(record);
    const before = await fs.readFile(settings);
    await assert.rejects(within(writeClaudeSettings(settings, state, {
      origin: 'http://127.0.0.1:18789', localToken: 'synthetic-parent-token',
    }), 'competing process did not reject'),
    error => error instanceof SettingsError && /locked/.test(error.message));
    assert.deepEqual(await fs.readFile(settings), before);
    assert.deepEqual(await fs.readFile(record), recordBefore);
    assert.equal((await fs.stat(`${record}.frm-lock`)).isDirectory(), true);
    assert.equal((await fs.stat(`${settings}.frm-lock`)).isDirectory(), true);
    child.send({ type: 'release' });
    await within(closed, 'released writer did not close');
    assert.deepEqual(exit, { code: 0, signal: null });
    assert.equal(stderr, '');
    assert.deepEqual(messages.map(message => message.type), ['holding', 'saved']);
    assert.equal(await undoClaudeSettings(state), 'restored');
    assert.deepEqual(JSON.parse(await fs.readFile(settings, 'utf8')), original);
    await assert.rejects(fs.stat(record), { code: 'ENOENT' });
    const entries = await fs.readdir(root, { recursive: true });
    assert.deepEqual(entries.filter(entry => entry.includes('.frm-')), []);
    await cleanup();
    await assert.rejects(fs.stat(root), { code: 'ENOENT' });
    context.diagnostic(JSON.stringify({ node: process.version, bundleSha256,
      separateWriter: ready.pid !== process.pid, competitorRejected: true, workerExit: exit,
      undo: 'restored', locksRemoved: true, privateStateRemoved: true }));
  });
