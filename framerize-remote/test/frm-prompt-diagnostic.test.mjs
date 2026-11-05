import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const { commandFailure } = await import(pathToFileURL(join(packageRoot, 'dist/index.js')).href);

async function bounded(promise, message) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), 3000);
    })]);
  } finally { clearTimeout(timer); }
}

const modes = [
  { name: 'setup', args: ['setup'] },
  { name: 'setup JSON', args: ['setup', '--json'], json: true },
  { name: 'client install', args: ['client', 'claude'], client: true },
];

test('native abort errors have the existing cancellation diagnostic', () => {
  const error = new DOMException('synthetic private input', 'AbortError');
  assert.deepEqual(JSON.parse(commandFailure(error, true)), {
    error: 'aborted', code: 'aborted', message: 'The command was cancelled.',
    next: 'Run the command again to retry.',
  });
  assert.equal(commandFailure(error, false),
    'The command was cancelled.\nNext: Run the command again to retry.');
});

test('unrecognized failures keep the generic fallback', () => {
  for (const error of [new Error('synthetic private input'), { name: 'AbortError' }, null]) {
    assert.equal(commandFailure(error, false), undefined);
    assert.equal(commandFailure(error, true), undefined);
  }
});

for (const mode of modes) for (const input of ['EOF', 'Ctrl+C']) {
  test(`${mode.name} reports cancellation on ${input}`, { timeout: 7000 }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'frm-prompt-diagnostic-'));
    const guard = join(root, 'terminal.cjs');
    let child, closed, stdout = '', stderr = '';
    context.after(async () => {
      try {
        if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
        if (closed) await bounded(closed, 'Launcher cleanup timed out');
      } finally { await rm(root, { recursive: true, force: true }); }
    });
    await writeFile(guard, `
const { appendFileSync } = require('node:fs');
const processes = require('node:child_process');
const denied = () => {
  appendFileSync(${JSON.stringify(join(root, 'unexpected-effect'))}, 'attempted\\n');
  throw new Error('Commands and network are forbidden in this fixture');
};
Object.defineProperty(process.stdin, 'isTTY', { value: true });
Object.defineProperty(process.stdout, 'isTTY', { value: true });
for (const name of ['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync', 'fork'])
  processes[name] = denied;
require('node:net').Socket.prototype.connect = denied;
globalThis.fetch = denied;
require('node:module').syncBuiltinESMExports();
`, { mode: 0o600 });
    const reached = Promise.withResolvers();
    child = spawn(process.execPath, ['--require', guard, join(packageRoot, 'bin/frm.js'), ...mode.args], {
      cwd: root, stdio: ['pipe', 'pipe', 'pipe'],
      env: { HOME: root, USERPROFILE: root, FRM_HOME: root, PATH: '',
        TMPDIR: root, TMP: root, TEMP: root, NODE_DISABLE_COMPILE_CACHE: '1',
        ...(mode.client ? { FRM_REMOTE: 'http://127.0.0.1:1' } : {}) },
    });
    closed = new Promise((resolve, reject) => {
      child.once('error', reject);
      child.once('close', (code, signal) => resolve({ code, signal }));
    });
    child.stdout.setEncoding('utf8').on('data', chunk => {
      stdout += chunk;
      if (stdout.includes(mode.client ? 'Type install to continue: ' : 'Server address, such as'))
        reached.resolve();
    });
    child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk; });
    await bounded(Promise.race([reached.promise, closed.then(exit => {
      throw new Error(`Launcher exited before the prompt: ${JSON.stringify(exit)} ${stderr}`);
    })]), 'Launcher did not reach its prompt');
    if (input === 'EOF') child.stdin.end();
    else child.stdin.write('\x03');
    assert.deepEqual(await bounded(closed, 'Prompt ignored cancellation'), { code: 1, signal: null });
    assert.deepEqual(await readdir(root), ['terminal.cjs'], 'No profile, client or command state may be written');
    assert.doesNotMatch(stdout, /Profile saved|Local API:/);
    if (mode.json) assert.deepEqual(JSON.parse(stderr), {
      error: 'aborted', code: 'aborted', message: 'The command was cancelled.',
      next: 'Run the command again to retry.',
    });
    else assert.equal(stderr, 'The command was cancelled.\nNext: Run the command again to retry.\n');
  });
}
