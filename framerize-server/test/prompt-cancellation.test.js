import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { stripVTControlCharacters } from 'node:util';

const packageRoot = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
const prompts = { access: '(lan): ', upstream: 'Upstream inference API URL: ' };
const cancelled = 'Server setup was cancelled.';

async function bounded(promise, message) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), 5000);
    })]);
  } finally { clearTimeout(timer); }
}

async function launch(context, args) {
  const root = await mkdtemp(join(tmpdir(), 'server-prompt-'));
  const home = join(root, 'home'), guard = join(root, 'terminal.cjs');
  let child, closed, stdout = '', stderr = '';
  context.after(async () => {
    try {
      if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
      if (closed) await bounded(closed, 'Launcher cleanup timed out');
    } finally { await rm(root, { recursive: true, force: true }); }
  });
  await mkdir(home);
  await writeFile(guard, `
const { appendFileSync, writeFileSync } = require('node:fs');
const denied = () => {
  appendFileSync(${JSON.stringify(join(root, 'unexpected-effect'))}, 'attempted\\n');
  throw new Error('Commands and network are forbidden in this fixture');
};
Object.defineProperty(process.stdin, 'isTTY', { value: true });
Object.defineProperty(process.stderr, 'isTTY', { value: true });
process.stderr.columns = 120;
const processes = require('node:child_process');
for (const name of ['spawn', 'spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync', 'fork'])
  processes[name] = denied;
require('node:net').Socket.prototype.connect = denied;
require('node:net').Server.prototype.listen = denied;
globalThis.fetch = denied;
require('node:module').syncBuiltinESMExports();
process.on('exit', () => writeFileSync(${JSON.stringify(join(root, 'exit.json'))},
  JSON.stringify({ sigint: process.listenerCount('SIGINT'), sigterm: process.listenerCount('SIGTERM') })));
`, { mode: 0o600 });
  const reached = { access: Promise.withResolvers(), upstream: Promise.withResolvers() };
  const reported = Promise.withResolvers();
  child = spawn(process.execPath, ['--require', guard, join(packageRoot, 'bin/framerize-server.js'), ...args], {
    cwd: home, stdio: ['pipe', 'pipe', 'pipe'],
    env: { HOME: home, USERPROFILE: home, FRAMERIZE_SERVER_HOME: join(home, 'server'),
      PATH: '', TMPDIR: root, TMP: root, TEMP: root, NODE_DISABLE_COMPILE_CACHE: '1' },
  });
  closed = new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => resolve({ code, signal }));
  });
  closed.catch(() => {});
  child.stdout.setEncoding('utf8').on('data', chunk => { stdout += chunk; });
  child.stderr.setEncoding('utf8').on('data', chunk => {
    stderr += chunk;
    if (stderr.includes('{"event":"server.failed"')) reported.resolve();
    for (const [name, text] of Object.entries(prompts)) {
      if (stderr.includes(text)) reached[name].resolve();
    }
  });
  return {
    child,
    async diagnostic() {
      await bounded(reported.promise, 'Prompt did not report its failure');
    },
    async prompt(name) {
      await bounded(Promise.race([reached[name].promise, closed.then(exit => {
        throw new Error(`Launcher exited before ${name}: ${JSON.stringify(exit)} ${stderr}`);
      })]), `Launcher did not reach ${name}`);
    },
    async finish(message = cancelled) {
      assert.deepEqual(await bounded(closed, 'Prompt ignored cancellation'), { code: 1, signal: null });
      assert.equal(stdout, '');
      assert.deepEqual(await readdir(home), [], 'Setup must not write configuration');
      assert.deepEqual((await readdir(root)).sort(), ['exit.json', 'home', 'terminal.cjs']);
      assert.deepEqual(JSON.parse(await readFile(join(root, 'exit.json'), 'utf8')),
        { sigint: 0, sigterm: 0 }, 'Setup must remove its signal handlers');
      const text = stripVTControlCharacters(stderr);
      assert.deepEqual(JSON.parse(text.slice(text.indexOf('{"event":'))), {
        event: 'server.failed', message,
        next: 'Run framerize status --json or inspect server.log in FRAMERIZE_SERVER_HOME.',
      });
    },
  };
}

for (const stage of ['access', 'upstream']) for (const input of ['Ctrl+C', 'Ctrl+D', 'EOF']) {
  test(`${stage} prompt reports cancellation on ${input}`, { timeout: 10000 }, async context => {
    const fixture = await launch(context, ['up', ...(stage === 'upstream' ? ['--no-lan'] : [])]);
    await fixture.prompt(stage);
    if (input === 'EOF') fixture.child.stdin.end();
    else {
      fixture.child.stdin.write(input === 'Ctrl+C' ? '\x03' : '\x04');
      await fixture.diagnostic();
      // A pipe stays open after readline closes. A real terminal does not need this EOF.
      fixture.child.stdin.end();
    }
    await fixture.finish();
  });
}

test('an answered access prompt continues to the upstream prompt', { timeout: 10000 }, async context => {
  const fixture = await launch(context, ['up']);
  await fixture.prompt('access');
  fixture.child.stdin.write('local\n');
  await fixture.prompt('upstream');
  fixture.child.stdin.end();
  await fixture.finish();
});

test('an answered upstream prompt keeps its validation error', { timeout: 10000 }, async context => {
  const fixture = await launch(context, ['up', '--no-lan']);
  await fixture.prompt('upstream');
  fixture.child.stdin.write('\n');
  await fixture.finish('An upstream inference API URL is required.');
});

for (const signal of ['SIGINT', 'SIGTERM']) {
  test(`an OS ${signal} keeps the setup cancellation diagnostic`, {
    skip: process.platform === 'win32', timeout: 10000,
  }, async context => {
    const fixture = await launch(context, ['up']);
    await fixture.prompt('access');
    fixture.child.kill(signal);
    await fixture.finish();
  });
}
