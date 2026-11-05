import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { EventEmitter, getEventListeners } from 'node:events';
import fs from 'node:fs';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { createConnection } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { PassThrough } from 'node:stream';
import { setTimeout as delay } from 'node:timers/promises';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/index.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { nodeRunner, offerClients, runClient } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const childEnv = { NODE_DISABLE_COMPILE_CACHE: '1',
  ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) };

async function bounded(promise, message) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), 2500);
    })]);
  } finally { clearTimeout(timer); }
}

function command(script) {
  return { command: process.execPath, args: ['-e', script], display: 'synthetic updater' };
}

function portState(port) {
  return new Promise(resolve => {
    const socket = createConnection({ host: '127.0.0.1', port });
    const finish = state => { socket.destroy(); resolve(state); };
    socket.once('connect', () => finish('open'));
    socket.once('error', error => finish(error.code));
    socket.setTimeout(200, () => finish('timeout'));
  });
}

async function portCloses(port) {
  const end = Date.now() + 1500;
  do {
    if (await portState(port) === 'ECONNREFUSED') return;
    await delay(10);
  } while (Date.now() < end);
  assert.fail('Owned updater listener did not close');
}

async function readState(path) {
  const end = Date.now() + 2500;
  do {
    try { return JSON.parse(await readFile(path, 'utf8')); }
    catch (error) { if (error.code !== 'ENOENT') throw error; }
    await delay(10);
  } while (Date.now() < end);
  assert.fail('Owned updater did not listen');
}

function updaterScript(mode, state) {
  const child = `
    const fs = require('node:fs');
    const listener = require('node:net').createServer(socket => socket.end());
    process.on('SIGTERM', () => { ${mode.includes('graceful')
      ? `setTimeout(() => listener.close(() => {
          fs.writeFileSync(${JSON.stringify(state + '.graceful')}, 'closed');
          process.exit(0);
        }), 100);` : ''} });
    listener.listen(0, '127.0.0.1', () => {
      const state = ${JSON.stringify(state)};
      fs.writeFileSync(state + '.tmp', JSON.stringify({ pid: process.pid, port: listener.address().port }));
      fs.renameSync(state + '.tmp', state);
    });
  `;
  if (!mode.startsWith('descendant')) return child;
  return `
    require('node:child_process').spawn(process.execPath, ['-e', ${JSON.stringify(child)}], { stdio: 'ignore' });
    process.on('SIGTERM', () => process.exit(0));
    setInterval(() => {}, 1000);
  `;
}

async function ownedFixture(context) {
  const root = await mkdtemp(join(tmpdir(), 'client-run-cancel-'));
  const fixture = { root, statePath: join(root, 'state.json'), child: null, state: null, wasClosed: false };
  fixture.runner = nodeRunner(process.platform, (...args) => {
    fixture.child = spawn(...args);
    fixture.closed = new Promise(resolve => fixture.child.once('close', () => {
      fixture.wasClosed = true;
      resolve();
    }));
    return fixture.child;
  });
  context.after(async () => {
    const errors = [];
    const attempt = async action => { try { await action(); } catch (error) { errors.push(error); } };
    await attempt(async () => {
      for (const pid of new Set([fixture.child?.pid, fixture.state?.pid])) {
        if (!pid) continue;
        try { process.kill(pid, 'SIGKILL'); }
        catch (error) { if (error.code !== 'ESRCH') throw error; }
      }
    });
    await attempt(async () => { if (fixture.closed) await bounded(fixture.closed, 'Child cleanup timed out'); });
    await attempt(async () => { if (fixture.state) await portCloses(fixture.state.port); });
    await attempt(() => rm(root, { recursive: true, force: true }));
    if (errors.length) throw new AggregateError(errors, 'Owned updater cleanup failed');
  });
  return fixture;
}

for (const mode of ['graceful', 'stubborn', 'descendant', 'descendant-graceful']) {
  test(`cancelled client command closes its ${mode} process before settling`, {
    timeout: 6000, skip: process.platform === 'win32' && 'POSIX process tree fixture',
  }, async context => {
    const fixture = await ownedFixture(context);
    const controller = new AbortController();
    const pending = fixture.runner.run(command(updaterScript(mode, fixture.statePath)),
      childEnv, controller.signal);
    fixture.state = await readState(fixture.statePath);
    assert.equal(await portState(fixture.state.port), 'open');
    if (mode.startsWith('descendant')) assert.notEqual(fixture.state.pid, fixture.child.pid);
    controller.abort();
    assert.equal(await bounded(pending, 'Cancelled updater did not settle'), 1);
    assert.equal(fixture.wasClosed, true);
    assert.equal(fixture.child.signalCode, mode === 'stubborn' ? 'SIGKILL' : null);
    await portCloses(fixture.state.port);
    if (mode.includes('graceful')) assert.equal(await readFile(fixture.statePath + '.graceful', 'utf8'), 'closed');
    assert.deepEqual(getEventListeners(controller.signal, 'abort'), []);
  });
}

test('an already cancelled client command never spawns', async () => {
  const controller = new AbortController();
  controller.abort();
  const runner = nodeRunner(process.platform, () => assert.fail('Unexpected spawn'));
  assert.equal(await runner.run(command(''), {}, controller.signal), 1);
});

test('cancellable maintenance can open its controlling terminal', {
  timeout: 8000, skip: process.platform !== 'linux' && 'Linux script accepts a piped stdin',
}, async context => {
  const root = await mkdtemp(join(tmpdir(), 'client-terminal-'));
  const pids = join(root, 'pids'), state = join(root, 'state');
  let terminal, closed;
  context.after(async () => {
    terminal?.kill('SIGKILL');
    for (const pid of (await readFile(pids, 'utf8').catch(() => '')).trim().split('\n').map(Number)) {
      if (pid > 0) try { process.kill(pid, 'SIGKILL'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
    }
    if (closed) await bounded(closed.catch(() => {}), 'Terminal cleanup timed out');
    await rm(root, { recursive: true, force: true });
  });
  await writeFile(join(root, 'runner.mjs'), compiled.outputFiles[0].contents);
  const probe = join(root, 'probe.mjs');
  await writeFile(probe, `
    import { spawn } from 'node:child_process';
    import { appendFileSync, writeFileSync } from 'node:fs';
    import { nodeRunner } from './runner.mjs';
    writeFileSync(${JSON.stringify(pids)}, process.pid + '\\n');
    const runner = nodeRunner(process.platform, (...args) => {
      const child = spawn(...args);
      appendFileSync(${JSON.stringify(pids)}, child.pid + '\\n');
      return child;
    });
    process.exitCode = await runner.run(${JSON.stringify(command(`
      const fs = require('node:fs');
      const fd = fs.openSync('/dev/tty', 'r+');
      fs.writeFileSync(${JSON.stringify(state)}, 'controlling terminal opened');
      fs.closeSync(fd);
    `))}, process.env, new AbortController().signal);
  `);
  const quote = value => `'${value.replaceAll("'", "'\\''")}'`;
  const args = ['-q', '-e', '-c', `${quote(process.execPath)} ${quote(probe)}`, '/dev/null'];
  terminal = spawn('/usr/bin/script', args, { cwd: root, env: { ...childEnv, TERM: 'xterm' } });
  let output = '';
  for (const stream of [terminal.stdout, terminal.stderr]) {
    stream.on('data', chunk => { output = (output + chunk).slice(-65536); });
  }
  closed = new Promise((resolve, reject) => {
    terminal.once('error', reject);
    terminal.once('close', resolve);
  });
  assert.equal(await bounded(closed, 'Terminal command did not close'), 0, output);
  assert.equal(await readFile(state, 'utf8'), 'controlling terminal opened');
});

test('standalone installation owns cancellation and removes its handlers', async () => {
  const signals = ['SIGINT', 'SIGTERM'];
  const before = signals.map(signal => process.listeners(signal));
  const errors = [];
  await assert.rejects(runClient(['claude', '--install', '--no-input'], {
    platform: 'linux', env: { PATH: '/synthetic/bin' }, home: '/synthetic/home',
    stateDir: '/synthetic/state', target: { origin: 'http://127.0.0.1:8788' },
    probe: () => false, io: { stdout() {}, stderr: text => errors.push(text) },
    runner: {
      async run(_command, _env, signal) {
        assert.ok(signal instanceof AbortSignal);
        const handlers = process.listeners('SIGINT').filter(handler => !before[0].includes(handler));
        assert.equal(handlers.length, 1);
        handlers[0]();
        assert.equal(signal.aborted, true);
        return 1;
      },
      async capture() { assert.fail('Cancelled installation must not probe or launch a client'); },
    },
  }), { name: 'AbortError' });
  assert.deepEqual(signals.map(signal => process.listeners(signal)), before);
  assert.deepEqual(errors, []);
});

test('a completed command keeps its exit code and releases its abort listener', async () => {
  const controller = new AbortController();
  const code = await nodeRunner().run(command('process.exit(7)'), childEnv, controller.signal);
  assert.equal(code, 7);
  assert.deepEqual(getEventListeners(controller.signal, 'abort'), []);
});

test('an interactive client launch does not receive maintenance cancellation', async () => {
  const controller = new AbortController();
  let launched = false;
  const code = await runClient(['codex'], {
    platform: 'linux', env: { PATH: '/synthetic/bin' }, home: '/synthetic/home',
    stateDir: '/synthetic/state', target: { origin: 'http://127.0.0.1:8788' },
    signal: controller.signal, probe: () => true, io: { stdout() {}, stderr() {} },
    runner: {
      async run(_command, _env, signal) { launched = true; assert.equal(signal, undefined); return 7; },
      async capture() { assert.fail('An interactive launch does not check versions'); },
    },
  });
  assert.equal(code, 7);
  assert.equal(launched, true);
});

test('simulated Windows cancellation waits for both the child and taskkill', async () => {
  const controller = new AbortController(), calls = [];
  const child = Object.assign(new EventEmitter(), { pid: 1234, kill() {} });
  const killer = new EventEmitter();
  const runner = nodeRunner('win32', (...args) => {
    calls.push(args);
    return calls.length === 1 ? child : killer;
  });
  let settled = false;
  const pending = runner.run(command(''), {}, controller.signal).then(code => { settled = true; return code; });
  controller.abort();
  assert.equal(calls[0][2].detached, false);
  assert.deepEqual(calls[1].slice(0, 2), ['taskkill', ['/PID', '1234', '/T', '/F']]);
  assert.equal(calls[1][2].timeout, 1000);
  child.emit('close', null, 'SIGKILL');
  await Promise.resolve();
  assert.equal(settled, false);
  killer.emit('close', 0);
  assert.equal(await pending, 1);
  assert.deepEqual(getEventListeners(controller.signal, 'abort'), []);
});

test('cancelling an offered installer suppresses failure output and the next client', async () => {
  const controller = new AbortController(), output = [], errors = [];
  const results = await offerClients({
    platform: 'linux', env: { PATH: '/synthetic/bin' }, home: '/synthetic/home',
    stateDir: '/synthetic/state', target: { origin: 'http://127.0.0.1:8788' },
    signal: controller.signal, probe: () => false, prompt: async () => 'install',
    io: { stdout: text => output.push(text), stderr: text => errors.push(text) },
    runner: {
      async run(_command, _env, signal) {
        assert.equal(signal, controller.signal);
        controller.abort();
        return 1;
      },
      async capture() { assert.fail('No installed client should be probed'); },
    },
  });
  assert.deepEqual(results, [{ client: 'claude', state: 'declined' }]);
  assert.deepEqual(errors, []);
  assert.doesNotMatch(output.join(''), /Codex|exited with code/);
});

test('terminal cancellation closes a reparented descendant before the terminal owner exits', {
  timeout: 10000, skip: process.platform !== 'linux' && 'Linux script accepts a piped stdin',
}, async context => {
  const root = await mkdtemp(join(tmpdir(), 'client-terminal-tree-'));
  const pids = join(root, 'pids'), statePath = join(root, 'state.json');
  let terminal, closed, state, terminalClosed = false;
  context.after(async () => {
    for (const pid of (await readFile(pids, 'utf8').catch(() => '')).trim().split('\n').map(Number)) {
      if (pid > 1) try { process.kill(pid, 'SIGKILL'); }
      catch (error) { if (error.code !== 'ESRCH') throw error; }
    }
    terminal?.kill('SIGKILL');
    if (closed) await bounded(closed.catch(() => {}), 'Terminal cleanup timed out');
    if (state) await portCloses(state.port);
    await rm(root, { recursive: true, force: true });
  });
  await writeFile(join(root, 'runner.mjs'), compiled.outputFiles[0].contents);
  await writeFile(join(root, 'worker.cjs'), `
    const fs = require('node:fs'), root = process.env.FRM_TERMINAL_ROOT;
    fs.appendFileSync(root + '/pids', process.pid + '\\n');
    for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => {});
    const listener = require('node:net').createServer(socket => socket.end());
    listener.listen(0, '127.0.0.1', () => {
      fs.closeSync(fs.openSync('/dev/tty', 'r+'));
      fs.writeFileSync(root + '/state.tmp', JSON.stringify({ pid: process.pid,
        port: listener.address().port, tty: process.stdin.isTTY && process.stdout.isTTY }));
      fs.renameSync(root + '/state.tmp', root + '/state.json');
    });
  `);
  await writeFile(join(root, 'installer.cjs'), `
    const fs = require('node:fs'), root = process.env.FRM_TERMINAL_ROOT;
    fs.appendFileSync(root + '/pids', process.pid + '\\n');
    process.on('SIGINT', () => { fs.writeFileSync(root + '/parent-signal', 'SIGINT'); process.exit(0); });
    process.on('SIGTERM', () => process.exit(0));
    require('node:child_process').spawn(process.execPath, [root + '/worker.cjs'], { stdio: 'inherit' });
  `);
  await writeFile(join(root, 'launcher.mjs'), `
    import cp from 'node:child_process';
    import fs from 'node:fs';
    import { syncBuiltinESMExports } from 'node:module';
    const root = process.env.FRM_TERMINAL_ROOT, inspect = cp.execFile;
    let direct;
    cp.execFile = (...args) => {
      if (args[0] !== '/bin/ps') throw new Error('Unexpected fixture command');
      if (direct?.exitCode === null && direct.signalCode === null) {
        direct.once('close', () => inspect(...args));
      } else return inspect(...args);
    };
    syncBuiltinESMExports();
    const { nodeRunner } = await import('./runner.mjs');
    const controller = new AbortController();
    for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => controller.abort());
    const runner = nodeRunner(process.platform, (...args) => {
      const child = cp.spawn(...args);
      fs.appendFileSync(root + '/pids', child.pid + '\\n');
      if (args[0] === process.execPath) direct = child;
      let group = '';
      child.stdio[3]?.on('data', chunk => {
        group += chunk;
        if (/^[1-9]\\d*\\n$/.test(group)) fs.appendFileSync(root + '/pids', group);
      });
      return child;
    });
    const code = await runner.run({ command: process.execPath,
      args: [root + '/installer.cjs'], display: 'synthetic installer' }, process.env, controller.signal);
    fs.writeFileSync(root + '/result.tmp', JSON.stringify({ code, aborted: controller.signal.aborted }));
    fs.renameSync(root + '/result.tmp', root + '/result.json');
    process.exitCode = code;
  `);
  await writeFile(join(root, 'owner.mjs'), `
    import { spawn } from 'node:child_process';
    import fs from 'node:fs';
    const root = process.env.FRM_TERMINAL_ROOT;
    for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => {});
    fs.appendFileSync(root + '/pids', process.pid + '\\n');
    const child = spawn(process.execPath, [root + '/launcher.mjs'], { stdio: 'inherit' });
    fs.appendFileSync(root + '/pids', child.pid + '\\n');
    child.once('close', (code, signal) => {
      fs.writeFileSync(root + '/exit.tmp', JSON.stringify({ code, signal }));
      fs.renameSync(root + '/exit.tmp', root + '/exit.json');
      process.stdin.once('data', chunk => {
        fs.writeFileSync(root + '/input.json', JSON.stringify(chunk.toString().trim()));
        process.stdin.pause();
      });
    });
    const timer = setInterval(() => { if (fs.existsSync(root + '/release')) clearInterval(timer); }, 10);
  `);
  const owner = join(root, 'owner.mjs');
  const quote = value => `'${value.replaceAll("'", "'\\''")}'`;
  const args = ['-q', '-e', '-c', `${quote(process.execPath)} ${quote(owner)}`, '/dev/null'];
  terminal = spawn('/usr/bin/script', args, {
    cwd: root, env: { ...childEnv, TERM: 'xterm', FRM_TERMINAL_ROOT: root },
  });
  let output = '';
  for (const stream of [terminal.stdout, terminal.stderr]) {
    stream.on('data', chunk => { output = (output + chunk).slice(-65536); });
  }
  closed = new Promise((resolve, reject) => {
    terminal.once('error', reject);
    terminal.once('close', code => { terminalClosed = true; resolve(code); });
  });
  state = await readState(statePath);
  assert.equal(state.tty, true);
  assert.equal(await portState(state.port), 'open');
  terminal.stdin.write('\x03');
  assert.deepEqual(await readState(join(root, 'result.json')), { code: 1, aborted: true });
  assert.deepEqual(await readState(join(root, 'exit.json')), { code: 1, signal: null });
  assert.equal(await readFile(join(root, 'parent-signal'), 'utf8'), 'SIGINT');
  assert.equal(terminalClosed, false, 'The persistent terminal owner must still be alive');
  assert.equal(await portState(state.port), 'ECONNREFUSED', 'Reparented installer descendant survived');
  terminal.stdin.write('restored\n');
  assert.equal(await readState(join(root, 'input.json')), 'restored');
  await writeFile(join(root, 'release'), 'release');
  assert.equal(await bounded(closed, 'Terminal owner did not close'), 0, output);
});

function terminalStartupFixture(context) {
  context.mock.method(fs, 'openSync', () => 123);
  context.mock.method(fs, 'closeSync', () => {});
  syncBuiltinESMExports();
  context.after(() => { context.mock.restoreAll(); syncBuiltinESMExports(); });
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const group = new PassThrough(), control = new PassThrough(), kills = [];
  const child = Object.assign(new EventEmitter(), { pid: 123456,
    stdio: [null, null, null, group, control],
    kill(signal) { kills.push(signal); child.emit('close', null, signal); return true; },
  });
  let decision = '';
  control.on('data', chunk => { decision += chunk; });
  const runner = nodeRunner('linux', () => child);
  return { runner, group, control, kills, decision: () => decision };
}

for (const mode of ['cancelled', 'silent', 'invalid']) {
  test(`a ${mode} terminal startup closes without running the installer`, async context => {
    const fixture = terminalStartupFixture(context), controller = new AbortController();
    const pending = fixture.runner.run(command(''), {}, controller.signal);
    if (mode === 'cancelled') controller.abort();
    else if (mode === 'invalid') fixture.group.write(process.pid + '\n');
    else context.mock.timers.tick(5000);
    context.mock.timers.tick(500);
    if (mode === 'cancelled') assert.equal(await pending, 1);
    else await assert.rejects(pending, mode === 'invalid'
      ? /Could not identify the client process group/ : /did not start in time/);
    assert.equal(fixture.decision(), 'stop\n');
    assert.deepEqual(fixture.kills, ['SIGKILL']);
    assert.equal(fixture.group.destroyed, true);
    assert.equal(fixture.control.destroyed, true);
    assert.deepEqual(getEventListeners(controller.signal, 'abort'), []);
  });
}
