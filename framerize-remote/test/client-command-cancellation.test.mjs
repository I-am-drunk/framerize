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

async function bounded(pending, message, milliseconds = 3000) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(message)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function readOptional(path) {
  try { return await readFile(path, 'utf8'); }
  catch (error) { if (error.code === 'ENOENT') return ''; throw error; }
}

const records = async path => (await readOptional(path)).trim().split('\n').filter(Boolean).map(JSON.parse);

function probeResponse(port) {
  return new Promise(resolveResponse => {
    let response = '';
    const socket = createConnection({ host: '127.0.0.1', port });
    const finish = result => { socket.destroy(); resolveResponse(result); };
    socket.setTimeout(400, () => finish('timeout'));
    socket.setEncoding('utf8').on('data', chunk => {
      response += chunk;
      if (response.length > 128) finish('oversized');
    });
    socket.once('end', () => finish(response));
    socket.once('error', error => finish(error.code));
  });
}

async function waitForProbe(path, running) {
  const deadline = Date.now() + 3000;
  while (Date.now() < deadline) {
    const state = await readOptional(path);
    if (state) return JSON.parse(state);
    assert.equal(running(), true, 'Launcher exited before the synthetic probe listened');
    await delay(10);
  }
  assert.fail('Synthetic version probe did not report its owned listener');
}

async function portClosed(port) {
  const deadline = Date.now() + 1500;
  do {
    if (await probeResponse(port) === 'ECONNREFUSED') return true;
    await delay(20);
  } while (Date.now() < deadline);
  return false;
}

function killCaptured(pid) {
  assert.ok(Number.isInteger(pid) && pid > 0 && pid !== process.pid);
  try { process.kill(pid, 'SIGKILL'); }
  catch (error) { if (error.code !== 'ESRCH') throw error; }
}

function executableSource() {
  return `#!${process.execPath}
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.appendFileSync(process.env.FRM_TEST_COMMANDS, JSON.stringify({pid: process.pid, args}) + '\\n');
if (args.length !== 1 || args[0] !== '--version') process.exit(79);
const listener = require('node:net').createServer(socket => socket.end('probe:' + process.pid));
listener.listen(0, '127.0.0.1', () => {
  const state = process.env.FRM_TEST_PROBE;
  fs.writeFileSync(state + '.tmp', JSON.stringify({pid: process.pid, port: listener.address().port}));
  fs.renameSync(state + '.tmp', state);
});
`;
}

function guardSource(executable, spawned, forbidden) {
  return `
const { appendFileSync } = require('node:fs');
const processes = require('node:child_process');
const reject = operation => {
  appendFileSync(${JSON.stringify(forbidden)}, JSON.stringify({operation}) + '\\n');
  throw new Error('Only the synthetic client may run in this fixture');
};
const launch = processes.spawn;
processes.spawn = (file, args, options) => {
  if (file !== ${JSON.stringify(executable)}) return reject('spawn');
  const child = launch(file, args, options);
  if (child.pid) appendFileSync(${JSON.stringify(spawned)}, JSON.stringify({pid: child.pid}) + '\\n');
  return child;
};
for (const method of ['spawnSync', 'exec', 'execSync', 'execFile', 'execFileSync', 'fork']) {
  processes[method] = () => reject(method);
}
globalThis.fetch = () => reject('fetch');
require('node:module').syncBuiltinESMExports();
`;
}

async function makeFixture(context) {
  const root = await mkdtemp(join(tmpdir(), 'frm-client-cancellation-'));
  const fixture = { root, bin: join(root, '.local', 'bin'), home: join(root, 'profile'),
    state: join(root, 'probe.json'), commands: join(root, 'commands.jsonl'),
    spawned: join(root, 'spawned.jsonl'), forbidden: join(root, 'forbidden.jsonl'),
    guard: join(root, 'guard.cjs'), child: undefined, closed: undefined, probe: undefined };
  fixture.running = () => fixture.child && fixture.child.exitCode === null
    && fixture.child.signalCode === null;
  context.after(async () => {
    const failures = [];
    const attempt = async action => { try { await action(); } catch (error) { failures.push(error); } };
    await attempt(async () => {
      if (fixture.running()) fixture.child.kill('SIGKILL');
      if (fixture.closed) await bounded(fixture.closed, 'Launcher cleanup timed out');
    });
    await attempt(async () => {
      for (const { pid } of await records(fixture.spawned)) killCaptured(pid);
    });
    await attempt(async () => {
      fixture.probe ??= JSON.parse((await readOptional(fixture.state)) || 'null');
      if (fixture.probe) assert.equal(await portClosed(fixture.probe.port), true,
        'Owned probe listener must close even when the baseline fails');
    });
    await attempt(() => rm(root, { recursive: true, force: true }));
    context.diagnostic(JSON.stringify({ cleanup: failures.length === 0,
      launcherClosed: !fixture.running(), probePortRefused: Boolean(fixture.probe) && failures.length === 0 }));
    if (failures.length) throw new AggregateError(failures, 'Owned fixture cleanup failed');
  });
  await mkdir(fixture.bin, { recursive: true, mode: 0o700 });
  await mkdir(fixture.home, { mode: 0o700 });
  fixture.profile = join(fixture.home, 'framerize-remote.json');
  fixture.original = Buffer.from('{\r\n  "remote": "http://127.0.0.1:9",\r\n'
    + '  "host": "127.0.0.1", "port": 8788\r\n}\r\n \t');
  fixture.executable = join(fixture.bin, 'claude');
  await writeFile(fixture.profile, fixture.original, { mode: 0o600 });
  await writeFile(fixture.executable, executableSource(), { mode: 0o700 });
  await writeFile(fixture.guard, guardSource(fixture.executable, fixture.spawned, fixture.forbidden),
    { mode: 0o600 });
  fixture.env = { HOME: root, USERPROFILE: root, PATH: fixture.bin, FRM_HOME: fixture.home,
    CLAUDE_CONFIG_DIR: join(root, 'claude'), FRM_TEST_PROBE: fixture.state,
    FRM_TEST_COMMANDS: fixture.commands, XDG_CONFIG_HOME: join(root, 'config'),
    XDG_DATA_HOME: join(root, 'data'), XDG_STATE_HOME: join(root, 'state'),
    XDG_CACHE_HOME: join(root, 'cache'), XDG_RUNTIME_DIR: join(root, 'run'),
    TMPDIR: root, TMP: root, TEMP: root, NODE_DISABLE_COMPILE_CACHE: '1' };
  return fixture;
}

for (const mode of ['--check', '--update']) {
  for (const signal of ['SIGINT', 'SIGTERM']) {
    test(`frm client claude ${mode} cancels its version probe on ${signal}`, {
      timeout: 15000, skip: process.platform === 'win32'
        ? 'Windows kill does not deliver catchable POSIX SIGINT/SIGTERM' : false,
    }, async context => {
      const fixture = await makeFixture(context);
      fixture.child = spawn(process.execPath, ['--require', fixture.guard, launcher,
        'client', 'claude', mode], {
        cwd: fixture.root, env: fixture.env, stdio: ['ignore', 'pipe', 'pipe'],
      });
      fixture.closed = new Promise((resolveExit, reject) => {
        fixture.child.once('error', reject);
        fixture.child.once('close', (code, exitSignal) => resolveExit({ code, signal: exitSignal }));
      });
      void fixture.closed.catch(() => {});
      let stdout = '', stderr = '', overflow = false;
      const capture = (channel, chunk) => {
        if (channel === 'stdout') stdout += chunk; else stderr += chunk;
        if (stdout.length + stderr.length > 65536) {
          overflow = true;
          if (fixture.running()) fixture.child.kill('SIGKILL');
        }
      };
      fixture.child.stdout.setEncoding('utf8').on('data', chunk => capture('stdout', chunk));
      fixture.child.stderr.setEncoding('utf8').on('data', chunk => capture('stderr', chunk));
      fixture.probe = await waitForProbe(fixture.state, fixture.running);
      const { pid, port } = fixture.probe;
      assert.deepEqual(await records(fixture.spawned), [{ pid }]);
      assert.deepEqual(await records(fixture.commands), [{ pid, args: ['--version'] }]);
      assert.equal(await probeResponse(port), `probe:${pid}`, 'Observe a live owned probe before signalling');
      assert.equal(fixture.running(), true);
      const beforeSignal = stdout.length;
      assert.equal(fixture.child.kill(signal), true);
      const exit = await bounded(fixture.closed, 'Launcher did not settle after cancellation');
      const closedBeforeCleanup = await portClosed(port);
      const commands = (await records(fixture.commands)).map(record => record.args);
      const spawnedPids = (await records(fixture.spawned)).map(record => record.pid);
      const forbidden = await records(fixture.forbidden);
      const unchangedProfile = fixture.original.equals(await readFile(fixture.profile));
      const observed = { exit, stderr, stdoutAfterSignal: stdout.slice(beforeSignal),
        commands, spawnedPids, forbidden, unchangedProfile, closedBeforeCleanup, overflow };
      context.diagnostic(JSON.stringify({ mode, signal, probe: { pid, port }, ...observed }));
      assert.deepEqual(observed, {
        exit: { code: 1, signal: null },
        stderr: 'The command was cancelled.\nNext: Run the command again to retry.\n',
        stdoutAfterSignal: '', commands: [['--version']], spawnedPids: [pid], forbidden: [],
        unchangedProfile: true, closedBeforeCleanup: true, overflow: false,
      });
    });
  }
}
