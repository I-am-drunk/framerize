import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { stripVTControlCharacters } from 'node:util';

const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const manifest = JSON.parse(await readFile(join(packageRoot, 'package.json'), 'utf8'));
assert.equal(typeof manifest.bin.frm, 'string');
const launcher = resolve(packageRoot, manifest.bin.frm);
const diagnostic = 'The command was cancelled.\nNext: Run the command again to retry.\n';

async function bounded(pending, message, milliseconds = 3000) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(message)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

function guardSource(attempts) {
  return `
const fs = require('node:fs');
Object.defineProperty(process.stdin, 'isTTY', { value: true });
Object.defineProperty(process.stdout, 'isTTY', { value: true });
Object.defineProperty(process.stdout, 'columns', { value: 120 });
const reject = operation => {
  fs.appendFileSync(${JSON.stringify(attempts)}, operation + '\\n');
  throw new Error('No process or network allowed during the address prompt');
};
for (const method of ['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync', 'fork']) {
  require('node:child_process')[method] = () => reject(method);
}
for (const module of ['http', 'https', 'net', 'tls', 'dns', 'dns/promises']) {
  for (const method of ['request', 'get', 'connect', 'createConnection', 'lookup', 'resolve']) {
    if (typeof require('node:' + module)[method] === 'function') {
      require('node:' + module)[method] = () => reject(module + '.' + method);
    }
  }
}
globalThis.fetch = () => reject('fetch');
require('node:module').syncBuiltinESMExports();
`;
}

async function fixture(context) {
  const root = await mkdtemp(join(tmpdir(), 'frm-address-prompt-'));
  const state = { root, attempts: join(root, 'attempts'), guard: join(root, 'guard.cjs') };
  context.after(async () => {
    try {
      if (state.child?.exitCode === null && state.child?.signalCode === null) state.child.kill('SIGKILL');
      if (state.closed) await bounded(state.closed, 'Launcher cleanup did not close');
    } finally { await rm(root, { recursive: true, force: true }); }
    context.diagnostic('Captured launcher closed; private temporary state removed.');
  });
  await writeFile(state.attempts, '', { mode: 0o600 });
  await writeFile(state.guard, guardSource(state.attempts), { mode: 0o600 });
  state.env = { HOME: root, USERPROFILE: root, PATH: root, FRM_HOME: join(root, 'profile'),
    XDG_CONFIG_HOME: join(root, 'config'), XDG_DATA_HOME: join(root, 'data'),
    XDG_STATE_HOME: join(root, 'state'), XDG_CACHE_HOME: join(root, 'cache'),
    XDG_RUNTIME_DIR: join(root, 'run'), TMPDIR: root, TMP: root, TEMP: root,
    NODE_DISABLE_COMPILE_CACHE: '1' };
  return state;
}

for (const command of ['setup', 'up']) {
  for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP']) {
    test(`frm ${command} cancels its initial address prompt on ${signal}`, {
      timeout: 10000, skip: process.platform === 'win32'
        ? 'Windows kill does not deliver these catchable POSIX signals' : false,
    }, async context => {
      const state = await fixture(context);
      let stdout = '';
      let stderr = '';
      let prompted;
      const prompt = new Promise(resolvePrompt => { prompted = resolvePrompt; });
      const expectedPrompt = command === 'setup'
        ? 'Server address, such as http://100.64.0.1:8789: ' : 'Pairing link or server address: ';
      state.child = spawn(process.execPath, ['--require', state.guard, launcher, command], {
        cwd: state.root, env: state.env, stdio: ['pipe', 'pipe', 'pipe'],
      });
      state.closed = new Promise((resolveClose, rejectClose) => {
        state.child.once('error', rejectClose);
        state.child.once('close', (code, exitSignal) => resolveClose({ code, signal: exitSignal }));
      });
      state.child.stdout.setEncoding('utf8').on('data', chunk => {
        stdout += chunk;
        if (stripVTControlCharacters(stdout).includes(expectedPrompt)) prompted();
      });
      state.child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk; });
      await bounded(Promise.race([prompt, state.closed.then(result => {
        throw new Error(`Launcher exited before its address prompt: ${JSON.stringify(result)} ${stderr}`);
      })]), 'Initial address prompt was not reached');
      assert.equal(state.child.exitCode, null);
      assert.equal(state.child.signalCode, null);
      assert.equal(state.child.kill(signal), true);
      const result = await bounded(state.closed, 'Cancellation did not close the launcher');
      const output = stripVTControlCharacters(stdout);
      const errorOutput = stripVTControlCharacters(stderr);
      const attempts = await readFile(state.attempts, 'utf8');
      const files = (await readdir(state.root, { recursive: true })).sort();
      context.diagnostic(JSON.stringify({ result, stdout: output, stderr: errorOutput, attempts, files }));
      assert.equal(attempts, '', 'No command, fetch, or socket may start');
      assert.deepEqual(files, ['attempts', 'guard.cjs'], 'No profile or service state may be written');
      assert.doesNotMatch(output, /Profile saved|This machine|"ready"\s*:\s*true|Next: frm serve/);
      assert.deepEqual({ ...result, stderr: errorOutput }, { code: 1, signal: null, stderr: diagnostic });
    });
  }
}
