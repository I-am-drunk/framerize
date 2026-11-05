import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { EventEmitter } from 'node:events';
import { mkdirSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { createServer, get } from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const packageRoot = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ? resolve(process.env.FRAMERIZE_SERVER_PACKAGE_ROOT)
  : fileURLToPath(new URL('../', import.meta.url));
const launcher = join(packageRoot, 'bin', 'framerize-server.js');
const { run } = await import(pathToFileURL(join(packageRoot, 'dist', 'cli.js')).href);

async function bounded(pending, message, milliseconds = 5000) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(message)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function listen(server) {
  await bounded(new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => {
      server.removeListener('error', reject);
      resolve();
    });
  }), 'Fixture did not start');
  return server.address().port;
}

async function closeFixture(server) {
  if (!server) return;
  server.closeAllConnections();
  if (server.listening) await bounded(new Promise((resolve, reject) => {
    server.close(error => error ? reject(error) : resolve());
  }), 'Fixture did not close');
}

function health(port) {
  return new Promise((resolve, reject) => {
    const request = get({ host: '127.0.0.1', port, path: '/health', agent: false,
      signal: AbortSignal.timeout(2000) }, response => {
      response.on('error', reject);
      response.on('end', () => resolve(response.statusCode));
      response.resume();
    });
    request.on('error', reject);
  });
}

test('up saves a working service, repeats without restart, and stops its real launcher',
  { timeout: 60_000 }, async context => {
    let root, upstream, reservation, child, closed;
    context.after(async () => {
      try {
        if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
        if (closed) await bounded(closed, 'Launcher cleanup timed out');
      } finally {
        try { await Promise.all([closeFixture(upstream), closeFixture(reservation)]); }
        finally { if (root) rmSync(root, { recursive: true, force: true }); }
      }
    });
    root = mkdtempSync(join(tmpdir(), 'server-up-recovery-'));
    const home = join(root, 'server');
    const env = { HOME: join(root, 'home'), USERPROFILE: join(root, 'home'),
      XDG_CONFIG_HOME: join(root, 'config'), XDG_DATA_HOME: join(root, 'data'),
      XDG_STATE_HOME: join(root, 'state'), XDG_CACHE_HOME: join(root, 'cache'),
      XDG_RUNTIME_DIR: join(root, 'run'), FRAMERIZE_SERVER_HOME: home,
      TMPDIR: join(root, 'tmp'), TMP: join(root, 'tmp'), TEMP: join(root, 'tmp'),
      PATH: '', NODE_DISABLE_COMPILE_CACHE: '1',
      ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}),
    };
    for (const path of new Set([home, env.HOME, env.XDG_CONFIG_HOME, env.XDG_DATA_HOME,
      env.XDG_STATE_HOME, env.XDG_CACHE_HOME, env.XDG_RUNTIME_DIR, env.TMPDIR])) {
      mkdirSync(path, { recursive: true, mode: 0o700 });
    }
    const requests = [];
    upstream = createServer((request, response) => {
      requests.push({ method: request.method, path: request.url, auth: request.headers.authorization });
      const models = request.method === 'GET' && request.url === '/v1/models';
      response.writeHead(models ? 200 : 404, { 'content-type': 'application/json' });
      response.end(JSON.stringify(models
        ? { object: 'list', data: [{ id: 'synthetic-model', object: 'model' }] } : { error: 'Not found' }));
    });
    const upstreamPort = await listen(upstream);
    const syntheticOrigin = `http://127.0.0.1:${upstreamPort}`;
    reservation = createServer();
    const port = await listen(reservation);
    await closeFixture(reservation);
    const origin = `http://127.0.0.1:${port}`;
    const configPath = join(home, 'server.json');
    const unitPath = join(env.XDG_CONFIG_HOME, 'systemd', 'user', 'framerize-server.service');
    let enabled = false;
    let restarts = 0;
    let stops = 0;
    let lastExit;
    let childOutput = '';
    const running = () => child && child.exitCode === null && child.signalCode === null;
    const stopChild = async () => {
      if (!running()) return;
      stops += 1;
      assert.equal(child.kill('SIGTERM'), true);
      lastExit = await bounded(closed, 'Service launcher did not stop');
    };
    const calls = [];
    const runStep = async (command, args) => {
      calls.push({ command, args: [...args] });
      if (command === 'loginctl') {
        assert.deepEqual(args, ['show-user', '1000', '--property=Linger', '--value']);
        return { code: 0, stdout: 'yes\n' };
      }
      assert.equal(command, 'systemctl', 'Only injected systemd commands are permitted');
      assert.equal(args[0], '--user');
      const action = args[1];
      if (action === 'show-environment' || action === 'daemon-reload') {
        assert.equal(args.length, 2);
        return { code: 0, stdout: '' };
      }
      assert.deepEqual(args.slice(2), action === 'disable'
        ? ['--now', 'framerize-server.service'] : ['framerize-server.service']);
      if (action === 'is-enabled') return { code: enabled ? 0 : 1, stdout: enabled ? 'enabled\n' : 'disabled\n' };
      if (action === 'is-active') return { code: running() ? 0 : 3, stdout: running() ? 'active\n' : 'inactive\n' };
      if (action === 'enable') enabled = true;
      else if (action === 'restart') {
        await stopChild();
        restarts += 1;
        child = spawn(process.execPath, [launcher, 'serve', '--config', configPath], {
          cwd: root, env, stdio: ['ignore', 'ignore', 'pipe'],
        });
        closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })));
        child.stderr.setEncoding('utf8').on('data', chunk => {
          childOutput = (childOutput + chunk).slice(-16_384);
        });
        await bounded(new Promise((resolve, reject) => {
          child.once('spawn', resolve);
          child.once('error', reject);
        }), 'Service launcher did not spawn');
      } else if (action === 'stop' || action === 'disable') {
        await stopChild();
        if (action === 'disable') enabled = false;
      } else assert.fail(`Unexpected injected systemctl action: ${action}`);
      return { code: 0, stdout: '' };
    };
    let output = '';
    let errors = '';
    const events = new EventEmitter();
    events.stdout = { write(text) { output = (output + text).slice(-16_384); return true; } };
    events.stderr = { write(text) { errors = (errors + text).slice(-16_384); return true; } };
    const dependencies = { env, process: events, setup: {
      program: [process.execPath, launcher], platform: 'linux', uid: 1000, run: runStep,
    } };
    const args = ['up', '--backend', 'http', '--upstream', syntheticOrigin,
      '--host', '127.0.0.1', '--port', String(port), '--no-lan', '--json'];
    const assertPairing = service => {
      const result = JSON.parse(output);
      assert.equal(result.ready, true);
      assert.equal(result.service, service);
      assert.equal(result.origin, origin);
      assert.equal(result.pairingUrl, `${origin}/pair`);
      assert.equal(errors, '');
    };
    assert.equal(await bounded(run(args, dependencies), 'First up did not finish', 20_000), 0);
    assertPairing('installed');
    assert.equal(restarts, 1);
    assert.equal(enabled, true);
    assert.ok(running(), 'The installed service must own a running launcher');
    assert.equal(await health(port), 200);
    const settingsBytes = readFileSync(configPath, 'utf8');
    const settings = JSON.parse(settingsBytes);
    assert.equal(settings.config.backend, 'http');
    assert.equal(settings.config.upstream, new URL(syntheticOrigin).href);
    assert.equal(settings.config.host, '127.0.0.1');
    assert.equal(settings.config.port, port);
    assert.equal(settings.config.privateNetworkOnly, false);
    const unitBytes = readFileSync(unitPath, 'utf8');
    assert.match(unitBytes, /^ExecStart=.+ serve --config /m);
    const installedChild = child;
    const installedPid = child.pid;
    const beforeRepeat = calls.length;
    output = '';
    assert.equal(await bounded(run(args, dependencies), 'Repeated up did not finish', 20_000), 0);
    assertPairing('unchanged');
    assert.equal(restarts, 1);
    assert.equal(stops, 0);
    assert.equal(child, installedChild);
    assert.equal(child.pid, installedPid);
    assert.ok(running(), 'Repeated up must leave the same launcher running');
    assert.ok(calls.slice(beforeRepeat).every(call => call.command === 'loginctl'
      || ['show-environment', 'is-enabled', 'is-active'].includes(call.args[1])),
    'Repeated up must not change the installed service');
    assert.equal(readFileSync(configPath, 'utf8'), settingsBytes);
    assert.equal(readFileSync(unitPath, 'utf8'), unitBytes);
    assert.equal(await health(port), 200);
    assert.ok(requests.length >= 2, 'Both up calls must reach real upstream readiness');
    for (const request of requests) {
      assert.deepEqual(request, { method: 'GET', path: '/v1/models', auth: undefined });
    }
    output = '';
    assert.equal(await bounded(run(['service', 'stop'], dependencies), 'Service stop did not finish'), 0);
    assert.equal(stops, 1);
    assert.equal(restarts, 1);
    assert.equal(await bounded(closed, 'Service close was not observed'), lastExit);
    if (process.platform === 'win32') assert.ok(lastExit.code === 0 || lastExit.signal === 'SIGTERM');
    else assert.deepEqual(lastExit, { code: 0, signal: null });
    await assert.rejects(health(port), { code: 'ECONNREFUSED' });
    assert.doesNotMatch(childOutput, /server\.failed/);
    assert.equal(errors, '');
    await closeFixture(upstream);
    await assert.rejects(health(upstreamPort), { code: 'ECONNREFUSED' });
    context.diagnostic('Injected service commands only; real launcher readiness, repeat and stop completed.');
  });
