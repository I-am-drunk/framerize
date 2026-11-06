import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const packageRoot = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
const quote = value => `'${value.replaceAll("'", "'\\''")}'`;
async function bounded(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Account terminal did not settle')), 3000);
    })]);
  } finally { clearTimeout(timer); }
}

async function launch(context) {
  const root = await mkdtemp(join(tmpdir(), 'account-terminal-'));
  const home = join(root, 'home'), guard = join(root, 'pid.cjs'), pidPath = join(root, 'pid');
  await mkdir(home);
  await writeFile(guard, `require('node:fs').writeFileSync(process.env.FIXTURE_PID, String(process.pid));`);
  const command = [process.execPath, '--require', guard, join(packageRoot, 'bin/framerize-server.js'),
    'accounts', 'add', 'private', '--project-id', 'project-private', '--select'].map(quote).join(' ');
  const child = spawn('/usr/bin/script', ['-q', '-e', '-c', `exec ${command}`, '/dev/null'], {
    cwd: home, env: { HOME: home, USERPROFILE: home, FRAMERIZE_SERVER_HOME: home,
      FIXTURE_PID: pidPath, PATH: '', TERM: 'xterm' },
  });
  const waiters = new Map();
  let output = '', done = false;
  const receive = chunk => {
    output += chunk;
    for (const [text, resolve] of waiters) if (output.includes(text)) resolve();
  };
  child.stdout.on('data', receive);
  child.stderr.on('data', receive);
  const closed = new Promise((resolve, reject) => {
    child.once('error', reject);
    child.once('close', code => { done = true; resolve(code); });
  });
  context.after(async () => {
    if (!done) {
      const pid = Number(await readFile(pidPath, 'utf8').catch(() => ''));
      if (pid) try { process.kill(pid, 'SIGKILL'); }
      catch (error) { if (error.code !== 'ESRCH') throw error; }
      child.kill('SIGKILL');
    }
    try { await bounded(closed); }
    finally { await rm(root, { recursive: true, force: true }); }
  });
  const prompt = 'Framer session (input hidden): ';
  if (!output.includes(prompt)) await bounded(new Promise(resolve => waiters.set(prompt, resolve)));
  return { child, home, output: () => output, async finish() {
    const code = await bounded(closed);
    const pid = Number(await readFile(pidPath, 'utf8'));
    assert.throws(() => process.kill(pid, 0), { code: 'ESRCH' });
    return code;
  } };
}

const options = { skip: process.platform !== 'linux', timeout: 8000 };

test('a real terminal hides session input and keeps backspace editing', options, async context => {
  const state = await launch(context);
  state.child.stdin.write('session=synthetic-hidden-valueX\x7f\n');
  assert.equal(await state.finish(), 0);
  const stored = JSON.parse(await readFile(join(state.home, 'accounts.json'), 'utf8'));
  assert.equal(stored[0].session, 'session=synthetic-hidden-value');
  assert.doesNotMatch(state.output(), /synthetic-hidden/);
  assert.equal(state.output().split('Framer session (input hidden): ').length, 2);
});

test('Ctrl+C cancels hidden input without saving a partial session', options, async context => {
  const state = await launch(context);
  state.child.stdin.write('session=synthetic-partial-value\x03');
  assert.equal(await state.finish(), 1);
  assert.deepEqual(await readdir(state.home), []);
  assert.doesNotMatch(state.output(), /synthetic-partial/);
  assert.match(state.output(), /Server setup was cancelled/);
});
