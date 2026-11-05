import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { EventEmitter } from 'node:events';
import { existsSync, mkdirSync, mkdtempSync, rmSync } from 'node:fs';
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

async function closeFixture(server) {
  if (!server) return;
  server.closeAllConnections();
  if (server.listening) await bounded(new Promise((resolve, reject) => {
    server.close(error => error ? reject(error) : resolve());
  }), 'Unrelated listener did not close');
}

function readiness(port) {
  return new Promise((resolve, reject) => {
    const request = get({ host: '127.0.0.1', port, path: '/ready', agent: false,
      signal: AbortSignal.timeout(2000) }, response => {
      let body = '';
      response.setEncoding('utf8').on('data', chunk => {
        body += chunk;
        if (body.length > 16_384) response.destroy(new Error('Fixture response exceeded its bound'));
      });
      response.on('error', reject);
      response.on('end', () => resolve({ status: response.statusCode, body }));
    });
    request.on('error', reject);
  });
}

test('up rejects an unrelated ready listener without leaving service state',
  { timeout: 60_000 }, async context => {
    let root, fixture, child, closed;
    context.after(async () => {
      try {
        if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
        if (closed) await bounded(closed, 'Launcher cleanup timed out');
      } finally {
        try { await closeFixture(fixture); }
        finally { if (root) rmSync(root, { recursive: true, force: true }); }
      }
    });
    root = mkdtempSync(join(tmpdir(), 'server-up-occupied-port-'));
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
    const readyBody = JSON.stringify({ status: 'ready', apiVersion: 1,
      capabilities: { responses: true, chatCompletions: true, streaming: true },
      fixture: 'unrelated-listener' });
    fixture = createServer((request, response) => {
      response.writeHead(request.url === '/ready' ? 200 : 404,
        { 'content-type': 'application/json' });
      response.end(request.url === '/ready' ? readyBody : '{}');
    });
    await bounded(new Promise((resolve, reject) => {
      fixture.once('error', reject);
      fixture.listen(0, '127.0.0.1', () => {
        fixture.removeListener('error', reject);
        resolve();
      });
    }), 'Unrelated listener did not start');
    const port = fixture.address().port;
    const origin = `http://127.0.0.1:${port}`;
    const expectedResponse = { status: 200, body: readyBody };
    assert.deepEqual(await readiness(port), expectedResponse);
    const configPath = join(home, 'server.json');
    const unitPath = join(env.XDG_CONFIG_HOME, 'systemd', 'user', 'framerize-server.service');
    assert.equal(existsSync(configPath), false);
    assert.equal(existsSync(unitPath), false);
    let enabled = false;
    let activations = 0;
    let childOutput = '';
    let lastExit;
    const running = () => Boolean(child && child.exitCode === null && child.signalCode === null);
    const stopChild = async () => {
      if (!running()) return;
      assert.equal(child.kill('SIGTERM'), true);
      lastExit = await bounded(closed, 'Service launcher did not stop');
      if (process.platform === 'win32') assert.ok(lastExit.code === 0 || lastExit.signal === 'SIGTERM');
      else assert.deepEqual(lastExit, { code: 0, signal: null });
    };
    const runStep = async (command, args) => {
      if (command === 'loginctl') {
        assert.deepEqual(args, ['show-user', '1000', '--property=Linger', '--value']);
        return { code: 0, stdout: 'yes\n' };
      }
      assert.equal(command, 'systemctl', 'Only injected Linux service commands are permitted');
      assert.equal(args[0], '--user');
      const action = args[1];
      if (action === 'show-environment' || action === 'daemon-reload') {
        assert.equal(args.length, 2);
        return { code: 0, stdout: '' };
      }
      if (action === 'show') {
        assert.deepEqual(args.slice(2), ['framerize-server.service', '--property=ActiveState,SubState,MainPID']);
        return { code: 0, stdout: running()
          ? `ActiveState=active\nSubState=running\nMainPID=${child.pid}\n`
          : 'ActiveState=inactive\nSubState=dead\nMainPID=0\n' };
      }
      assert.deepEqual(args.slice(2), action === 'disable'
        ? ['--now', 'framerize-server.service'] : ['framerize-server.service']);
      if (action === 'is-enabled') return { code: enabled ? 0 : 1, stdout: enabled ? 'enabled\n' : 'disabled\n' };
      if (action === 'is-active') return { code: running() ? 0 : 3, stdout: running() ? 'active\n' : 'inactive\n' };
      if (action === 'enable') enabled = true;
      else if (action === 'restart') {
        await stopChild();
        activations += 1;
        child = spawn(process.execPath, [launcher, 'serve', '--config', configPath], {
          cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'],
        });
        closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })));
        for (const stream of [child.stdout, child.stderr]) {
          stream.setEncoding('utf8').on('data', chunk => {
            childOutput = (childOutput + chunk).slice(-16_384);
          });
        }
        await bounded(new Promise((resolve, reject) => {
          child.once('spawn', resolve);
          child.once('error', reject);
        }), 'Service launcher did not spawn');
        lastExit = await bounded(closed, 'Occupied-port launcher did not close', 10_000);
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
    const args = ['up', '--backend', 'http', '--upstream', origin,
      '--host', '127.0.0.1', '--port', String(port), '--no-lan', '--json'];
    const outcome = await bounded(run(args, dependencies), 'Occupied-port up did not finish', 25_000)
      .then(code => ({ code }), error => ({ error }));
    const retained = { config: existsSync(configPath), unit: existsSync(unitPath), enabled };
    assert.equal(fixture.listening, true, 'up must leave the unrelated listener running');
    assert.deepEqual(await readiness(port), expectedResponse);
    await closeFixture(fixture);
    await assert.rejects(readiness(port), { code: 'ECONNREFUSED' });
    const messages = output.split('\n').filter(line => line.trim()).map(line => JSON.parse(line));
    const childEvents = childOutput.split('\n').filter(line => line.startsWith('{')).map(line => JSON.parse(line));
    const childFailure = childEvents.find(event => event.event === 'server.failed');
    const observed = { rejected: outcome.error !== undefined,
      readyOutput: messages.some(message => message.ready === true),
      pairingOutput: messages.some(message => typeof message.pairingUrl === 'string'), ...retained };
    context.diagnostic(JSON.stringify({ observed, returnedCode: outcome.code, activations,
      childExit: lastExit ?? null, childFailure: childFailure ? {
        event: childFailure.event, message: childFailure.message, next: childFailure.next,
      } : null, errorOutputBytes: Buffer.byteLength(errors),
      unrelatedListenerPreserved: true, fixturePortReleased: true }));
    if (child) {
      assert.deepEqual(lastExit, { code: 1, signal: null });
      assert.ok(childFailure, 'The occupied-port launcher must report its startup failure');
      assert.ok(childEvents.every(event => event.event !== 'server.listening'));
      assert.equal(running(), false, 'The captured launcher must have closed');
    }
    if (outcome.error !== undefined) {
      assert.ok(outcome.error instanceof Error);
      assert.match(outcome.error.message, /already in use/i);
    }
    assert.deepEqual(observed, { rejected: true, readyOutput: false, pairingOutput: false,
      config: false, unit: false, enabled: false },
    'up must reject an occupied port without success output or newly installed service state');
  });
