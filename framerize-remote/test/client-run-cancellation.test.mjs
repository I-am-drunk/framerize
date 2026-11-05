import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { EventEmitter, getEventListeners } from 'node:events';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { createConnection } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
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
    process.on('SIGTERM', () => { ${mode === 'graceful'
      ? 'setTimeout(() => listener.close(() => process.exit(0)), 100);' : ''} });
    listener.listen(0, '127.0.0.1', () => {
      const state = ${JSON.stringify(state)};
      fs.writeFileSync(state + '.tmp', JSON.stringify({ pid: process.pid, port: listener.address().port }));
      fs.renameSync(state + '.tmp', state);
    });
  `;
  if (mode !== 'descendant') return child;
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
      if (fixture.child?.pid) {
        try { process.kill(-fixture.child.pid, 'SIGKILL'); }
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

for (const mode of ['graceful', 'stubborn', 'descendant']) {
  test(`cancelled client command closes its ${mode} process before settling`, {
    timeout: 6000, skip: process.platform === 'win32' && 'POSIX process group fixture',
  }, async context => {
    const fixture = await ownedFixture(context);
    const controller = new AbortController();
    const pending = fixture.runner.run(command(updaterScript(mode, fixture.statePath)),
      childEnv, controller.signal);
    fixture.state = await readState(fixture.statePath);
    assert.equal(await portState(fixture.state.port), 'open');
    if (mode === 'descendant') assert.notEqual(fixture.state.pid, fixture.child.pid);
    controller.abort();
    assert.equal(await bounded(pending, 'Cancelled updater did not settle'), 1);
    assert.equal(fixture.wasClosed, true);
    assert.equal(fixture.child.signalCode, mode === 'stubborn' ? 'SIGKILL' : null);
    await portCloses(fixture.state.port);
    assert.deepEqual(getEventListeners(controller.signal, 'abort'), []);
  });
}

test('an already cancelled client command never spawns', async () => {
  const controller = new AbortController();
  controller.abort();
  const runner = nodeRunner(process.platform, () => assert.fail('Unexpected spawn'));
  assert.equal(await runner.run(command(''), {}, controller.signal), 1);
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
