import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, lstat, readdir, open, rm } from 'node:fs/promises';
import { constants } from 'node:fs';
import { spawn, execFileSync } from 'node:child_process';
import { join, dirname } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/settings.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const bytes = compiled.outputFiles[0].contents;
const moduleUrl = `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`;
const sourceDigest = createHash('sha256').update(bytes).digest('hex');
const childSource = `
  import fs from 'node:fs/promises';
  import { syncBuiltinESMExports } from 'node:module';
  const [moduleUrl, action, settings, state, watched] = process.argv.slice(1);
  const original = fs.readFile;
  fs.readFile = (...args) => {
    if (args[0] === watched) process.send({ event: 'read-entered' });
    return original(...args);
  };
  syncBuiltinESMExports();
  const api = await import(moduleUrl);
  try {
    const result = action === 'undo' ? await api.undoClaudeSettings(state)
      : await api.writeClaudeSettings(settings, state, { origin: 'http://127.0.0.1:12345' });
    process.send({ event: 'result', result }, () => process.disconnect());
  } catch (error) {
    process.send({ event: 'result', error: { name: error.name, message: error.message } },
      () => process.disconnect());
  }
`;

async function within(promise, milliseconds, label) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(label)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

for (const action of ['persist', 'undo']) {
  test(`${action} rejects nonregular persistence input without awaiting a FIFO peer`,
    { timeout: 6000, skip: process.platform === 'win32' }, async context => {
      const root = await mkdtemp(join(tmpdir(), 'frm-persistence-fifo-'));
      let child;
      let closed;
      let exit;
      const cleanup = async () => {
        if (child && !exit) { child.kill('SIGKILL'); await within(closed, 1500, 'owned child did not close'); }
        await rm(root, { recursive: true, force: true });
      };
      context.after(cleanup);
      const state = join(root, 'state');
      const settings = join(root, 'settings.json');
      const record = join(state, 'claude-settings.json');
      const watched = action === 'persist' ? settings : record;
      const original = '{"model":"synthetic-model","env":{"KEEP":"synthetic-value"}}\n';
      await mkdir(state, { mode: 0o700 });
      if (action === 'undo') await writeFile(settings, original, { mode: 0o600 });
      execFileSync('mkfifo', ['-m', '600', watched], { timeout: 1000 });
      const before = await lstat(watched);
      const entered = Promise.withResolvers();
      const messages = [];
      let stderr = '';
      child = spawn(process.execPath, ['--input-type=module', '-e', childSource,
        moduleUrl, action, settings, state, watched], {
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
      await within(Promise.race([entered.promise, closed]), 2000, 'no read entry or child close');
      let settledBeforeCleanup = false;
      try { await within(closed, 750, 'persistence read remained pending'); settledBeforeCleanup = true; }
      catch (error) { if (error.message !== 'persistence read remained pending') throw error; }
      const observedExit = exit;
      if (!exit) { child.kill('SIGKILL'); await within(closed, 1500, 'owned child did not close'); }
      const after = await lstat(watched);
      const entries = await readdir(state);
      const settingsAfter = action === 'undo' ? await readFile(settings, 'utf8') : undefined;
      let writerError;
      try {
        const handle = await open(watched, constants.O_WRONLY | constants.O_NONBLOCK);
        await handle.close();
      } catch (error) { writerError = error.code; }
      await cleanup();
      context.diagnostic(JSON.stringify({ node: process.version, sourceDigest, action,
        readEntered: messages.some(message => message.event === 'read-entered'),
        settledBeforeCleanup, observedExit, cleanupExit: exit, fifoWriterError: writerError,
        results: messages.filter(message => message.event === 'result'), privateStateRemoved: true }));
      assert.equal(stderr, '');
      assert.equal(after.isFIFO(), true);
      assert.equal(after.ino, before.ino);
      assert.deepEqual(entries, action === 'undo' ? ['claude-settings.json'] : []);
      if (action === 'undo') assert.equal(settingsAfter, original);
      assert.equal(writerError, 'ENXIO', 'no owned FIFO reader may remain');
      assert.equal(settledBeforeCleanup, true, 'nonregular input must settle before cleanup kills the child');
      assert.deepEqual(observedExit, { code: 0, signal: null });
      const result = messages.find(message => message.event === 'result');
      assert.ok(result, 'the function must report its result');
      if (action === 'persist') assert.match(result.error?.message ?? '', /settings/i);
      else assert.ok(result.error || result.result === 'not_configured');
    });
}

test('regular settings still persist and undo while preserving unrelated values',
  { timeout: 2000 }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'frm-persistence-control-'));
    context.after(() => rm(root, { recursive: true, force: true }));
    const settings = join(root, 'settings.json');
    const state = join(root, 'state');
    const original = { model: 'synthetic-model', env: { KEEP: 'synthetic-value' } };
    await writeFile(settings, JSON.stringify(original), { mode: 0o600 });
    const api = await import(moduleUrl);
    await api.writeClaudeSettings(settings, state, { origin: 'http://127.0.0.1:12345' });
    const written = JSON.parse(await readFile(settings, 'utf8'));
    const result = await api.undoClaudeSettings(state);
    const restored = JSON.parse(await readFile(settings, 'utf8'));
    const remaining = await readdir(state);
    await rm(root, { recursive: true, force: true });
    assert.equal(written.env.ANTHROPIC_BASE_URL, 'http://127.0.0.1:12345/anthropic');
    assert.equal(written.env.KEEP, original.env.KEEP);
    assert.equal(result, 'restored');
    assert.deepEqual(restored, original);
    assert.deepEqual(remaining, []);
    context.diagnostic('regular control passed; private state removed');
  });
