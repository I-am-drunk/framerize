import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const guard = new URL('./helpers/client-signal-preload.mjs', import.meta.url).href;
const quote = value => `'${value.replaceAll("'", "'\\''")}'`;
async function bounded(promise, message) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), 3000);
    })]);
  } finally { clearTimeout(timer); }
}

test('terminal Ctrl+C reaches an ordinary client once and leaves it usable', {
  skip: process.platform !== 'linux' && 'Uses Linux script for a real controlling terminal',
  timeout: 12000,
}, async t => {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'frm-client-terminal-')));
  const bin = join(root, 'bin'), state = join(root, 'state'), client = join(bin, 'codex');
  const trace = join(root, 'signals.jsonl');
  let terminal, closed, clientState, terminalClosed = false;
  t.after(async () => {
    try {
      if (!terminalClosed) {
        const recorded = (await readFile(trace, 'utf8').catch(() => '')).trim().split('\n')
          .filter(Boolean).map(JSON.parse);
        const pids = new Set(recorded.filter(event => event.kind === 'child' || event.kind === 'wrapper')
          .map(event => event.pid));
        for (const pid of pids) {
          if (pid) try { process.kill(pid, 'SIGKILL'); }
          catch (error) { if (error.code !== 'ESRCH') throw error; }
        }
        terminal?.kill('SIGKILL');
        if (closed) await bounded(closed, 'Terminal cleanup timed out');
      }
    } finally { await rm(root, { recursive: true, force: true }); }
  });
  await Promise.all([bin, state].map(path => mkdir(path)));
  const source = await readFile(new URL('./helpers/client-signal-fixture.cjs', import.meta.url), 'utf8');
  await writeFile(client, `#!${process.execPath}\n${source}`, { mode: 0o700 });
  const profilePath = join(state, 'framerize-remote.json');
  const profile = JSON.stringify({ remote: 'http://127.0.0.1:9', port: 8788 });
  await writeFile(profilePath, profile, { mode: 0o600 });
  const ready = Promise.withResolvers(), interrupted = Promise.withResolvers();
  const results = [];
  let output = '', pending = '';
  const command = ['exec', quote(process.execPath), '--import', quote(guard),
    quote(join(packageRoot, 'bin/frm.js')), 'client', 'codex', '--no-input'].join(' ');
  terminal = spawn('/usr/bin/script', ['-q', '-e', '-c', command, '/dev/null'], {
    cwd: root, env: { HOME: root, USERPROFILE: root, FRM_HOME: state, PATH: bin,
      FRM_TEST_CLIENT: client, FRM_TEST_SIGNALS: trace, TERM: 'xterm', NODE_DISABLE_COMPILE_CACHE: '1' },
  });
  closed = new Promise((resolve, reject) => {
    terminal.once('error', reject);
    terminal.once('close', code => { terminalClosed = true; resolve(code); });
  });
  terminal.stdout.setEncoding('utf8').on('data', chunk => {
    output = (output + chunk).slice(-65536);
    pending += chunk;
    const lines = pending.split('\n');
    pending = lines.pop();
    for (const line of lines) {
      try {
        const event = JSON.parse(line.slice(line.indexOf('{')).trim());
        if (event.kind === 'ready') { clientState = event; ready.resolve(event); }
        if (event.kind === 'interrupt') interrupted.resolve(event);
        if (event.kind === 'result') results.push(event);
      } catch {}
    }
  });
  terminal.stderr.on('data', chunk => { output = (output + chunk).slice(-65536); });
  await bounded(Promise.race([ready.promise, closed.then(code => {
    throw new Error(`Terminal closed before readiness: ${code} ${output}`);
  })]), 'Client did not become ready');
  assert.equal(clientState.tty, true);
  terminal.stdin.write('\x03');
  assert.deepEqual(await bounded(interrupted.promise, 'Ctrl+C did not reach the client'),
    { kind: 'interrupt', count: 1 });
  assert.equal(terminalClosed, false, 'Ctrl+C must not force the interactive client to exit');
  terminal.stdin.write('continue\n');
  assert.equal(await bounded(closed, 'Client did not close after input'), 0, output);
  assert.deepEqual(results, [{ kind: 'result', count: 1 }]);
  const events = (await readFile(trace, 'utf8')).trim().split('\n').map(JSON.parse);
  assert.deepEqual(events.filter(event => event.kind === 'forward' && event.signal === 'SIGINT'), []);
  assert.deepEqual(events.filter(event => event.kind === 'closed'), [{ kind: 'closed', code: 0, signal: null }]);
  assert.equal(await readFile(profilePath, 'utf8'), profile);
  t.diagnostic('Real terminal Ctrl+C delivered once; the client accepted later input and closed.');
});
