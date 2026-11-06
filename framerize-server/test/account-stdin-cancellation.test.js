import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const packageRoot = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
async function bounded(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Account input did not settle')), 1500);
    })]);
  } finally { clearTimeout(timer); }
}

async function launch(context) {
  const root = await mkdtemp(join(tmpdir(), 'account-stdin-'));
  const home = join(root, 'home'), hook = join(root, 'stdin.cjs');
  await mkdir(home);
  await writeFile(hook, `
const original = process.stdin[Symbol.asyncIterator].bind(process.stdin);
process.stdin[Symbol.asyncIterator] = function () {
  process.send('reading-stdin');
  process.disconnect();
  return original();
};
`);
  const child = spawn(process.execPath, ['--require', hook,
    join(packageRoot, 'bin/framerize-server.js'), 'accounts', 'add', 'private',
    '--project-id', 'project-private', '--session-stdin', '--select'], {
    stdio: ['pipe', 'pipe', 'pipe', 'ipc'], cwd: home,
    env: { HOME: home, USERPROFILE: home, FRAMERIZE_SERVER_HOME: home, PATH: '' },
  });
  let output = '', error = '';
  child.stdout.on('data', chunk => { output += chunk; });
  child.stderr.on('data', chunk => { error += chunk; });
  const closed = new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => resolve({ code, signal }));
  });
  context.after(async () => {
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    try { await bounded(closed); }
    finally { await rm(root, { recursive: true, force: true }); }
  });
  await bounded(new Promise(resolve => child.once('message', resolve)));
  return { child, closed, home, output: () => output, error: () => error };
}

for (const signal of ['SIGINT', 'SIGTERM']) {
  test(`account stdin stops on ${signal} while its writer stays open`, {
    skip: process.platform === 'win32', timeout: 6000,
  }, async context => {
    const state = await launch(context);
    state.child.kill(signal);
    assert.deepEqual(await bounded(state.closed), { code: 1, signal: null });
    assert.equal(state.output(), '');
    assert.equal(JSON.parse(state.error()).message, 'Server setup was cancelled.');
    assert.deepEqual(await readdir(state.home), []);
  });
}

test('account stdin accepts a completed pipe', { timeout: 6000 }, async context => {
  const state = await launch(context);
  state.child.stdin.end('session=synthetic-complete-pipe\n');
  assert.deepEqual(await bounded(state.closed), { code: 0, signal: null });
  assert.equal(state.error(), '');
  assert.match(state.output(), /Added private/);
  assert.deepEqual((await readdir(state.home)).sort(), ['accounts.json', 'server.json']);
  assert.doesNotMatch(state.output(), /synthetic-complete-pipe/);
});
