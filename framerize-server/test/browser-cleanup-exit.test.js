import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn, execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { build } from 'esbuild';

const exec = promisify(execFile);
const entry = fileURLToPath(new URL('../src/browser-login.ts', import.meta.url));
const helper = new URL('./helpers/browser-cleanup-exit-caller.fixture', import.meta.url);
const compiled = await build({ entryPoints: [entry], bundle: true, platform: 'node', format: 'esm', write: false });
const bytes = compiled.outputFiles[0].contents;
console.log(JSON.stringify({ runtime: process.version, bundle: createHash('sha256').update(bytes).digest('hex') }));
const sleep = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds));
async function bounded(promise, label, milliseconds = 7000) {
  const error = new Error(`Browser exit fixture timed out: ${label}`);
  let timer;
  try { return await Promise.race([promise, new Promise((resolve, reject) => {
    timer = setTimeout(() => reject(error), milliseconds);
  })]); } finally { clearTimeout(timer); }
}
async function running(pid) {
  try {
    const { stdout } = await exec('/bin/ps', ['-p', String(pid), '-o', 'stat='], { timeout: 500 });
    return stdout.trim().split(/\s+/).some(state => state && !/^[ZX]/.test(state));
  } catch (error) {
    if (error.code === 1 && !error.stdout?.trim() && !error.stderr?.trim()) return false;
    throw error;
  }
}
for (const mode of ['denied', 'pipe-close', 'graceful']) {
  test(`caller exits after browser cleanup: ${mode}`, {
    timeout: 12000, skip: process.platform === 'win32',
  }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'browser-exit-case-'));
    let caller, browserPid, closed, cleaned = false;
    const cleanup = async () => {
      if (cleaned) return;
      cleaned = true;
      try {
        if (browserPid) {
          try { process.kill(-browserPid, 'SIGKILL'); }
          catch (error) { if (error.code !== 'ESRCH') throw error; }
        }
        if (caller?.exitCode === null && caller.signalCode === null) caller.kill('SIGKILL');
        if (closed) await bounded(closed, 'fixture caller stop');
        if (browserPid) {
          const deadline = Date.now() + 1500;
          while (await running(browserPid) && Date.now() < deadline) await sleep(10);
          assert.equal(await running(browserPid), false, 'owned browser must stop');
        }
      } finally { await rm(root, { recursive: true, force: true }); }
    };
    context.after(cleanup);
    const bundle = join(root, 'browser.mjs');
    await writeFile(bundle, bytes);
    const callerPath = join(root, 'caller.mjs');
    await writeFile(callerPath, await readFile(helper));
    caller = spawn(process.execPath, [callerPath, bundle, root, mode], {
      cwd: root, env: { HOME: root, TMPDIR: root },
      stdio: ['ignore', 'pipe', 'pipe', 'ipc'],
    });
    let stderr = '', resolveOutcome;
    const outcome = new Promise(resolve => { resolveOutcome = resolve; });
    caller.stderr.on('data', chunk => { stderr += chunk; });
    caller.stdout.resume();
    caller.on('message', message => {
      if (message.kind === 'browser') browserPid = message.pid;
      if (message.kind === 'outcome') resolveOutcome(message);
    });
    closed = new Promise((resolve, reject) => {
      caller.once('error', reject);
      caller.once('close', (code, signal) => resolve({ code, signal }));
    });
    const result = await bounded(Promise.race([outcome, closed.then(value => {
      throw new Error(`Caller closed before reporting: ${JSON.stringify(value)} ${stderr}`);
    })]), 'cleanup diagnostic');
    assert.ok(Number.isSafeInteger(browserPid) && browserPid > 1);
    assert.equal(result.code, mode === 'graceful' ? 'cancelled' : 'cleanup_failed');
    assert.equal(result.profileRemoved, true);
    assert.equal(stderr, '');
    if (mode !== 'graceful') assert.ok(result.attempts.length > 0);
    const alive = await running(browserPid);
    const exit = await Promise.race([closed, sleep(750).then(() => null)]);
    await cleanup();
    context.diagnostic(JSON.stringify({ mode, result, browserAliveBeforeCleanup: alive, callerExit: exit }));
    assert.equal(alive, mode !== 'graceful');
    assert.deepEqual(exit, { code: mode === 'graceful' ? 130 : 1, signal: null });
  });
}
