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
const aborted = 'The command was cancelled.\nNext: Run the command again to retry.\n';

async function bounded(pending, message) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolveUnused, reject) => {
      timer = setTimeout(() => reject(new Error(message)), 3000);
    })]);
  } finally { clearTimeout(timer); }
}

function guardSource(attempts) {
  return `const fs = require('node:fs');
Object.defineProperty(process.stdin, 'isTTY', { value: true });
Object.defineProperty(process.stdout, 'isTTY', { value: true });
Object.defineProperty(process.stdout, 'columns', { value: 120 });
const reject = operation => {
  fs.appendFileSync(${JSON.stringify(attempts)}, operation + '\\n');
  throw new Error('No command or network is permitted in the consent fixture');
};
for (const method of ['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync', 'fork']) {
  require('node:child_process')[method] = () => reject(method);
}
for (const name of ['http', 'https', 'net', 'tls', 'dns', 'dns/promises']) {
  const module = require('node:' + name);
  for (const method of ['request', 'get', 'connect', 'createConnection', 'lookup', 'resolve']) {
    if (typeof module[method] === 'function') module[method] = () => reject(name + '.' + method);
  }
}
globalThis.fetch = () => reject('fetch');
require('node:module').syncBuiltinESMExports();
`;
}

async function fixture(context) {
  const root = await mkdtemp(join(tmpdir(), 'frm-install-consent-'));
  const state = { root, attempts: join(root, 'attempts'), guard: join(root, 'guard.cjs') };
  context.after(async () => {
    try {
      if (state.child?.exitCode === null && state.child?.signalCode === null) state.child.kill('SIGKILL');
      if (state.closed) await bounded(state.closed, 'Launcher cleanup did not close');
    } finally { await rm(root, { recursive: true, force: true }); }
    context.diagnostic('Captured launcher closed; private fixture removed.');
  });
  await writeFile(state.attempts, '', { mode: 0o600 });
  await writeFile(state.guard, guardSource(state.attempts), { mode: 0o600 });
  state.env = { HOME: root, USERPROFILE: root, PATH: root, FRM_HOME: join(root, 'profile'),
    FRM_REMOTE: 'http://127.0.0.1:9', XDG_CONFIG_HOME: join(root, 'config'),
    XDG_STATE_HOME: join(root, 'state'), XDG_DATA_HOME: join(root, 'data'),
    XDG_CACHE_HOME: join(root, 'cache'), TMPDIR: root, TMP: root, TEMP: root,
    NODE_DISABLE_COMPILE_CACHE: '1' };
  return state;
}

for (const [client, label] of [['claude', 'Claude Code'], ['codex', 'Codex']]) {
  for (const action of ['SIGINT', 'SIGTERM', 'decline']) {
    test(`missing ${client} install consent handles ${action}`, {
      timeout: 10000, skip: process.platform === 'win32' ? 'POSIX signal fixture' : false,
    }, async context => {
      const state = await fixture(context);
      let stdout = '';
      let stderr = '';
      let prompted;
      const prompt = new Promise(resolvePrompt => { prompted = resolvePrompt; });
      state.child = spawn(process.execPath, ['--require', state.guard, launcher, 'client', client], {
        cwd: state.root, env: state.env, stdio: ['pipe', 'pipe', 'pipe'],
      });
      state.closed = new Promise((resolveClose, rejectClose) => {
        state.child.once('error', rejectClose);
        state.child.once('close', (code, signal) => resolveClose({ code, signal }));
      });
      state.child.stdout.setEncoding('utf8').on('data', chunk => {
        stdout += chunk;
        if (stripVTControlCharacters(stdout).includes(`Install ${label}? Type install to continue: `)) prompted();
      });
      state.child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk; });
      await bounded(Promise.race([prompt, state.closed.then(result => {
        throw new Error(`Launcher exited before consent: ${JSON.stringify(result)} ${stderr}`);
      })]), 'Missing-client consent prompt was not reached');
      assert.equal(state.child.exitCode, null);
      assert.equal(state.child.signalCode, null);
      if (action === 'decline') state.child.stdin.write('no\n');
      else assert.equal(state.child.kill(action), true);
      const result = await bounded(state.closed, 'Launcher did not close after consent response');
      const output = stripVTControlCharacters(stdout);
      const errorOutput = stripVTControlCharacters(stderr);
      const attempts = await readFile(state.attempts, 'utf8');
      const files = (await readdir(state.root, { recursive: true })).sort();
      context.diagnostic(JSON.stringify({ result, errorOutput, attempts, files }));
      assert.equal(attempts, '', 'No installer, client, service, DNS or network call is permitted');
      assert.deepEqual(files, ['attempts', 'guard.cjs'], 'No profile or service state may be written');
      assert.doesNotMatch(output, /Profile saved|This machine|"ready"\s*:\s*true/);
      if (action === 'decline') {
        assert.deepEqual(result, { code: 3, signal: null });
        assert.equal(errorOutput,
          `${label} was not installed. Rerun with --install or FRM_CLIENT_INSTALL=1 to consent.\n`);
      } else {
        assert.deepEqual({ ...result, stderr: errorOutput }, { code: 1, signal: null, stderr: aborted });
      }
    });
  }
}
