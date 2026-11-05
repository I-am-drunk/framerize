import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createConnection } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';

const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const manifest = JSON.parse(await readFile(join(packageRoot, 'package.json'), 'utf8'));
assert.equal(typeof manifest.bin?.frm, 'string');
const launcher = resolve(packageRoot, manifest.bin.frm);
const aborted = 'The command was cancelled.\nNext: Run the command again to retry.\n';

async function bounded(pending, milliseconds = 3000) {
  let timer;
  try {
    return await Promise.race([pending, new Promise(resolveTimeout => {
      timer = setTimeout(() => resolveTimeout({ timedOut: true }), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function optionalText(path) {
  try { return await readFile(path, 'utf8'); }
  catch (error) { if (error.code === 'ENOENT') return ''; throw error; }
}
const records = async path => (await optionalText(path)).trim().split('\n').filter(Boolean).map(JSON.parse);

function responseFrom(port) {
  return new Promise(resolveResponse => {
    let response = '';
    const socket = createConnection({ host: '127.0.0.1', port });
    const finish = result => { socket.destroy(); resolveResponse(result); };
    socket.setTimeout(300, () => finish('timeout'));
    socket.setEncoding('utf8').on('data', chunk => {
      response += chunk;
      if (response.length > 128) finish('oversized');
    });
    socket.once('end', () => finish(response));
    socket.once('error', error => finish(error.code));
  });
}

async function eventually(predicate, message, milliseconds = 3000) {
  const deadline = Date.now() + milliseconds;
  do {
    const result = await predicate();
    if (result) return result;
    await delay(10);
  } while (Date.now() < deadline);
  assert.fail(message);
}

function killOwned(pid) {
  assert.ok(Number.isSafeInteger(pid) && pid > 0 && pid !== process.pid);
  try { process.kill(pid, 'SIGKILL'); }
  catch (error) { if (error.code !== 'ESRCH') throw error; }
}

function executableSource() {
  return `#!${process.execPath}
const fs = require('node:fs');
const args = process.argv.slice(2);
const record = event => fs.appendFileSync(process.env.FRM_TEST_EVENTS,
  JSON.stringify({event, pid: process.pid, args}) + '\\n');
if (args.length === 1 && args[0] === '--version') {
  record('version');
  process.stdout.write('2.1.300 (Synthetic Claude)\\n');
} else if (args.length === 1 && args[0] === 'update') {
  record('update');
  process.on('SIGTERM', () => record('ignored-SIGTERM'));
  process.on('SIGINT', () => record('ignored-SIGINT'));
  const listener = require('node:net').createServer(socket => socket.end('updater:' + process.pid));
  listener.listen(0, '127.0.0.1', () => {
    const path = process.env.FRM_TEST_STATE;
    fs.writeFileSync(path + '.tmp', JSON.stringify({pid: process.pid, port: listener.address().port}));
    fs.renameSync(path + '.tmp', path);
  });
} else {
  record('unexpected-command');
  process.exitCode = 79;
}
`;
}

function guardSource(executable, events) {
  return `
const { appendFileSync } = require('node:fs');
const processes = require('node:child_process');
const record = data => appendFileSync(${JSON.stringify(events)}, JSON.stringify(data) + '\\n');
const reject = operation => {
  record({event: 'forbidden', operation});
  throw new Error('Unexpected external operation in synthetic updater fixture');
};
const launch = processes.spawn;
const execFile = processes.execFile;
processes.spawn = (file, args, options) => {
  if (file !== ${JSON.stringify(executable)} || args.length !== 1
    || !['--version', 'update'].includes(args[0])) return reject('spawn');
  const child = launch(file, args, options);
  record({event: 'spawn', pid: child.pid, args});
  child.once('close', (code, signal) => record({event: 'close', pid: child.pid, args, code, signal}));
  return child;
};
for (const method of ['spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync', 'fork']) {
  processes[method] = (...args) => {
    if (method === 'execFile' && args[0] === '/bin/ps'
      && JSON.stringify(args[1]) === JSON.stringify(['-A', '-o', 'pid=,ppid='])
      && args[2]?.timeout === 1000 && args[2]?.killSignal === 'SIGKILL') {
      record({event: 'process-snapshot'});
      return execFile(...args);
    }
    return reject(method);
  };
}
let fetched = false;
globalThis.fetch = async url => {
  if (fetched || String(url) !== 'https://downloads.claude.ai/claude-code-releases/latest') return reject('fetch');
  fetched = true;
  record({event: 'fetch', url: String(url)});
  return new Response('2.1.301\\n');
};
require('node:module').syncBuiltinESMExports();
`;
}

async function makeFixture(context) {
  const root = await mkdtemp(join(tmpdir(), 'frm-update-process-'));
  const fixture = { root, bin: join(root, '.local', 'bin'), home: join(root, 'profile'),
    events: join(root, 'events.jsonl'), state: join(root, 'updater.json'),
    guard: join(root, 'guard.cjs'), child: undefined, closed: undefined, updater: undefined };
  fixture.running = () => fixture.child && fixture.child.exitCode === null && fixture.child.signalCode === null;
  context.after(async () => {
    const failures = [];
    const attempt = async action => { try { await action(); } catch (error) { failures.push(error); } };
    await attempt(async () => {
      const history = await records(fixture.events);
      for (const { pid } of history.filter(entry => entry.event === 'spawn')) {
        if (!history.some(entry => entry.event === 'close' && entry.pid === pid)) killOwned(pid);
      }
    });
    await attempt(async () => {
      if (fixture.running()) fixture.child.kill('SIGKILL');
      if (fixture.closed) assert.equal((await bounded(fixture.closed)).timedOut, undefined);
    });
    let portRefused = false;
    await attempt(async () => {
      fixture.updater ??= JSON.parse((await optionalText(fixture.state)) || 'null');
      if (fixture.updater) portRefused = await eventually(async () =>
        await responseFrom(fixture.updater.port) === 'ECONNREFUSED', 'Updater survived owned cleanup');
    });
    await attempt(() => rm(root, { recursive: true, force: true }));
    context.diagnostic(JSON.stringify({ cleanup: failures.length === 0,
      launcherClosed: !fixture.running(), updaterPortRefused: portRefused }));
    if (failures.length) throw new AggregateError(failures, 'Owned updater cleanup failed');
  });
  return fixture;
}

async function prepareFixture(fixture) {
  const { root } = fixture;
  await mkdir(fixture.bin, { recursive: true, mode: 0o700 });
  await mkdir(fixture.home, { mode: 0o700 });
  fixture.profile = join(fixture.home, 'framerize-remote.json');
  fixture.original = Buffer.from('{\r\n  "remote": "http://127.0.0.1:9",\r\n'
    + '  "host": "127.0.0.1", "port": 8788\r\n}\r\n \t');
  fixture.executable = join(fixture.bin, 'claude');
  await writeFile(fixture.profile, fixture.original, { mode: 0o600 });
  await writeFile(fixture.executable, executableSource(), { mode: 0o700 });
  await writeFile(fixture.guard, guardSource(fixture.executable, fixture.events), { mode: 0o600 });
  fixture.env = { HOME: root, USERPROFILE: root, PATH: fixture.bin, FRM_HOME: fixture.home,
    CLAUDE_CONFIG_DIR: join(root, 'claude'), FRM_TEST_STATE: fixture.state,
    FRM_TEST_EVENTS: fixture.events, XDG_CONFIG_HOME: join(root, 'config'),
    XDG_DATA_HOME: join(root, 'data'), XDG_STATE_HOME: join(root, 'state'),
    XDG_CACHE_HOME: join(root, 'cache'), XDG_RUNTIME_DIR: join(root, 'run'),
    TMPDIR: root, TMP: root, TEMP: root, NODE_DISABLE_COMPILE_CACHE: '1' };
}

function startLauncher(fixture) {
  fixture.child = spawn(process.execPath, ['--require', fixture.guard, launcher,
    'client', 'claude', '--update', '--no-input'], {
    cwd: fixture.root, env: fixture.env, stdio: ['ignore', 'pipe', 'pipe'],
  });
  fixture.closed = new Promise((resolveExit, reject) => {
    fixture.child.once('error', reject);
    fixture.child.once('close', (code, signal) => resolveExit({ code, signal }));
  });
  void fixture.closed.catch(() => {});
}

for (const signal of ['SIGINT', 'SIGTERM']) {
  test(`frm client update stops its active updater on ${signal}`, {
    timeout: 15000, skip: process.platform === 'win32'
      ? 'Windows kill does not deliver catchable POSIX signals' : false,
  }, async context => {
    const fixture = await makeFixture(context);
    await prepareFixture(fixture);
    startLauncher(fixture);
    let stdout = '', stderr = '', overflow = false;
    const capture = (channel, chunk) => {
      if (overflow) return;
      if (channel === 'stdout') stdout += chunk; else stderr += chunk;
      if (stdout.length + stderr.length > 65536) {
        overflow = true;
        if (fixture.running()) fixture.child.kill('SIGKILL');
      }
    };
    fixture.child.stdout.setEncoding('utf8').on('data', chunk => capture('stdout', chunk));
    fixture.child.stderr.setEncoding('utf8').on('data', chunk => capture('stderr', chunk));
    fixture.updater = await eventually(async () => {
      const state = await optionalText(fixture.state);
      if (state) return JSON.parse(state);
      assert.equal(fixture.running(), true, 'Launcher exited before updater startup');
      return false;
    }, 'Synthetic updater did not report its listener');
    const { pid, port } = fixture.updater;
    assert.equal(await responseFrom(port), `updater:${pid}`);
    await eventually(() => stdout.includes(`Update command: ${fixture.executable} update\n`),
      'Update command output was not received before signalling');
    const initial = await records(fixture.events);
    assert.deepEqual(initial.filter(entry => entry.event === 'forbidden'), []);
    const starts = initial.filter(entry => entry.event === 'spawn');
    assert.deepEqual(starts.map(entry => entry.args), [['--version'], ['update']]);
    assert.equal(starts[1].pid, pid);
    assert.ok(initial.some(entry => entry.event === 'close' && entry.pid === starts[0].pid
      && entry.code === 0 && entry.signal === null), 'Version probe completed successfully');
    assert.deepEqual(initial.filter(entry => entry.event === 'fetch'), [{ event: 'fetch',
      url: 'https://downloads.claude.ai/claude-code-releases/latest' }]);
    assert.ok(initial.some(entry => entry.event === 'update' && entry.pid === pid));
    assert.equal(fixture.running(), true);
    const beforeSignal = stdout.length;
    const started = Date.now();
    assert.equal(fixture.child.kill(signal), true);
    const exit = await bounded(fixture.closed);
    const elapsedMs = Date.now() - started;
    const listenerBeforeCleanup = await responseFrom(port);
    const history = await records(fixture.events);
    const updaterClose = history.find(entry => entry.event === 'close' && entry.pid === pid);
    const observed = { exit, stderr, stdoutAfterSignal: stdout.slice(beforeSignal),
      listenerBeforeCleanup, updaterClose: updaterClose ?? null,
      furtherOperations: history.slice(initial.length).filter(entry =>
        ['spawn', 'fetch', 'forbidden', 'unexpected-command', 'version', 'update'].includes(entry.event)),
      profileUnchanged: fixture.original.equals(await readFile(fixture.profile)), overflow };
    context.diagnostic(JSON.stringify({ signal, elapsedMs, pid, port, before: initial,
      ignoredSignals: history.filter(entry => entry.event.startsWith('ignored-')), ...observed }));
    assert.deepEqual(observed, { exit: { code: 1, signal: null }, stderr: aborted,
      stdoutAfterSignal: '', listenerBeforeCleanup: 'ECONNREFUSED',
      updaterClose: { event: 'close', pid, args: ['update'], code: null, signal: 'SIGKILL' },
      furtherOperations: [], profileUnchanged: true, overflow: false });
  });
}
