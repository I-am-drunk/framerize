import test from 'node:test';
import assert from 'node:assert/strict';
import { once, getEventListeners } from 'node:events';
import { createServer, Server } from 'node:http';
import { createConnection } from 'node:net';
import { mkdtemp, mkdir, readFile, writeFile, stat, readdir, rm, link, symlink } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { run } from '../dist/index.js';

const ready = { apiVersion: 1, status: 'ready',
  capabilities: { responses: true, chatCompletions: true, streaming: true } };
const setup = origin => ['setup', '--remote', origin, '--server-token-env', 'FIRST_SERVER',
  '--local-token-env', 'FIRST_LOCAL', '--port', '0'];
const quiet = { stdout() {}, stderr() {} };

test('failed startup output closes the listener and removes signal handlers', async context => {
  await fixture(context); const remote = await peer(context);
  await run(setup(remote.origin), quiet);
  const listeners = ['SIGINT', 'SIGTERM'].map(name => getEventListeners(process, name).length);
  let port;
  await assert.rejects(run(['serve', '--json'], { ...quiet, stdout(text) {
    port = JSON.parse(text).listening.port;
    throw new Error('synthetic output failure');
  } }), /synthetic output failure/);
  await refused(port);
  assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => getEventListeners(process, name).length), listeners);
});

test('cancelling startup closes readiness without opening the local listener', { timeout: 5000 }, async context => {
  await fixture(context);
  const reservation = createServer();
  reservation.listen(0, '127.0.0.1'); await once(reservation, 'listening');
  const port = reservation.address().port;
  await new Promise(resolve => reservation.close(resolve));
  let observedError; let output = '';
  const remote = createServer((request, response) => {
    request.resume(); response.write('{');
    void refused(port).catch(error => { observedError = error; })
      .finally(() => process.emit('SIGTERM'));
  });
  remote.listen(0, '127.0.0.1'); await once(remote, 'listening');
  context.after(async () => {
    remote.closeAllConnections(); await new Promise(resolve => remote.close(resolve));
  });
  const listeners = ['SIGINT', 'SIGTERM'].map(name => getEventListeners(process, name).length);
  const args = setup(`http://127.0.0.1:${remote.address().port}`).slice(0, -2);
  assert.equal(await run([...args, '--port', String(port), '--start'], {
    ...quiet, stdout(text) { output += text; },
  }), 0);
  assert.equal(observedError, undefined);
  assert.deepEqual(output.trim().split('\n'), ['Profile saved.', 'Stopped.']);
  assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => getEventListeners(process, name).length), listeners);
  await refused(port);
});

test('actual launcher reports argument errors with actionable, redacted diagnostics', async context => {
  const { root, profile } = await fixture(context);
  await mkdir(join(root, 'profile'));
  const saved = 'synthetic-profile-canary';
  await writeFile(profile, saved);
  const launcher = fileURLToPath(new URL('../bin/framerize-remote.js', import.meta.url));
  const launch = args => spawnSync(process.execPath, [launcher, ...args], {
    cwd: root, encoding: 'utf8', timeout: 5000,
    env: { HOME: root, USERPROFILE: root, TMPDIR: root, NODE_DISABLE_COMPILE_CACHE: '1',
      FRAMERIZE_REMOTE_HOME: join(root, 'profile') },
  });
  const message = 'The command options are invalid, incomplete or repeated.';
  const next = 'Run framerize-remote help for supported commands and options.';
  for (const command of ['check', 'serve']) {
    const cases = [
      ['unknown option', ['--synthetic-argument-canary']],
      ['positional argument', ['synthetic-argument-canary']],
      ['missing value', [command === 'serve' ? '--port' : '--config']],
      ['duplicate option', command === 'serve'
        ? ['--port', '0', '--port', '1'] : ['--tailscale', '--tailscale']],
    ];
    for (const json of [false, true]) for (const [name, args] of cases) {
      await context.test(`${command}, ${name}, ${json ? 'JSON' : 'text'}`, () => {
        const result = launch([command, ...args, ...(json ? ['--json'] : [])]);
        assert.equal(result.error, undefined); assert.equal(result.status, 1);
        assert.equal(result.stdout, '');
        if (json) assert.deepEqual(JSON.parse(result.stderr), {
          error: 'command_arguments', code: 'command_arguments', message, next,
        });
        else assert.equal(result.stderr, `${message}\nNext: ${next}\n`);
        assert.doesNotMatch(result.stderr, /canary/);
      });
    }
  }
  assert.equal(await readFile(profile, 'utf8'), saved);
  const help = launch(['help']);
  assert.equal(help.error, undefined); assert.equal(help.status, 0);
  assert.equal(help.stderr, ''); assert.match(help.stdout, /Usage:/);
});

for (const json of [false, true]) {
  test(`setup starts and stops with ${json ? 'JSON' : 'compact'} output`, { timeout: 5000 }, async context => {
    await fixture(context); const remote = await peer(context);
    const listeners = ['SIGINT', 'SIGTERM'].map(name => getEventListeners(process, name).length);
    let output = ''; let port; let prompts = 0;
    const code = await run([...setup(remote.origin), '--start', ...(json ? ['--json'] : [])], {
      async ask() { prompts += 1; return 'no'; },
      ...quiet, stdout(text) {
        output += text;
        const event = json ? JSON.parse(text) : undefined;
        const match = text.match(/Local API: http:\/\/127\.0\.0\.1:(\d+)\/v1/);
        if (event?.listening || match) {
          port = event?.listening.port ?? Number(match[1]); process.emit('SIGTERM');
        }
      },
    });
    assert.equal(code, 0); assert.equal(remote.calls(), 1); await refused(port);
    assert.equal(prompts, 0, 'stopped setup must not offer client installation');
    const lines = output.trim().split('\n');
    if (json) assert.deepEqual(lines.map(JSON.parse), [
      { saved: true }, { ready: true, listening: { host: '127.0.0.1', port } }, { stopped: true },
    ]);
    else assert.deepEqual(lines, ['Profile saved.', 'Remote readiness passed.',
      `Local API: http://127.0.0.1:${port}/v1`, 'Stop: Ctrl+C', 'Stopped.']);
    assert.doesNotMatch(output, /synthetic-(?:local|server)/);
    assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => getEventListeners(process, name).length), listeners);
  });
}

for (const signal of ['SIGINT', 'SIGTERM']) {
  test(`${signal} retains stop handlers until Remote cleanup finishes`, { timeout: 5000 }, async context => {
    await fixture(context); const remote = await peer(context);
    await run(setup(remote.origin), quiet);
    const signals = ['SIGINT', 'SIGTERM'];
    const baseline = signals.map(name => process.listenerCount(name));
    const entered = Promise.withResolvers();
    const originalClose = Server.prototype.close;
    let releaseClose; let closes = 0; let port;
    context.mock.method(Server.prototype, 'close', function(callback) {
      if (this === remote.server) return originalClose.call(this, callback);
      closes += 1;
      return originalClose.call(this, (...args) => {
        releaseClose = () => callback(...args);
        entered.resolve();
      });
    });
    const output = [];
    const serving = run(['serve', '--json'], { ...quiet, stdout(text) {
      const event = JSON.parse(text); output.push(event);
      if (event.listening) { port = event.listening.port; process.emit(signal); }
    } });
    await entered.promise;
    const during = signals.map(name => process.listenerCount(name));
    const handled = [...signals, signal].map(name => process.emit(name));
    releaseClose();
    assert.equal(await serving, 0);
    await refused(port);
    assert.equal(closes, 1);
    assert.equal(remote.calls(), 1);
    assert.deepEqual(during, baseline.map(count => count + 1));
    assert.deepEqual(handled, [true, true, true]);
    assert.deepEqual(signals.map(name => process.listenerCount(name)), baseline);
    assert.deepEqual(output, [
      { ready: true, listening: { host: '127.0.0.1', port } }, { stopped: true },
    ]);
  });
}

for (const json of [false, true]) {
  for (const signal of ['SIGINT', 'SIGTERM']) {
    test(`${signal} removes stop handlers when Remote cleanup fails with ${json ? 'JSON' : 'compact'} output`,
      { timeout: 5000 }, async context => {
      await fixture(context); const remote = await peer(context);
      await run(setup(remote.origin), quiet);
      const signals = ['SIGINT', 'SIGTERM'];
      const baseline = signals.map(name => process.listenerCount(name));
      const closed = Promise.withResolvers();
      const originalClose = Server.prototype.close;
      let closes = 0; let port; let text = '';
      context.mock.method(Server.prototype, 'close', function(callback) {
        if (this === remote.server) return originalClose.call(this, callback);
        closes += 1;
        originalClose.call(this, () => closed.resolve());
        throw new Error('synthetic cleanup failure');
      });
      await assert.rejects(run(['serve', ...(json ? ['--json'] : [])], { ...quiet, stdout(chunk) {
        text += chunk;
        if (json) {
          const event = JSON.parse(chunk);
          if (event.listening) { port = event.listening.port; process.emit(signal); }
        } else {
          const match = chunk.match(/Local API: http:\/\/127\.0\.0\.1:(\d+)\/v1/);
          if (match) { port = Number(match[1]); process.emit(signal); }
        }
      } }), { code: 'cleanup_failed', message: 'Remote cleanup failed.' });
      await closed.promise;
      await refused(port);
      assert.equal(closes, 1);
      assert.deepEqual(signals.map(name => process.listenerCount(name)), baseline);
      if (json) {
        assert.deepEqual(text.trim().split('\n').map(JSON.parse), [
          { ready: true, listening: { host: '127.0.0.1', port } },
        ]);
      } else {
        assert.doesNotMatch(text, /Stopped\./);
        assert.deepEqual(text.trim().split('\n'), [
          'Remote readiness passed.',
          `Local API: http://127.0.0.1:${port}/v1`,
          'Stop: Ctrl+C',
        ]);
      }
    });
  }
}

for (const signal of ['SIGINT', 'SIGTERM']) {
  test(`${signal} handles repeated stops during a Remote check`, { timeout: 5000 }, async context => {
    await fixture(context);
    const signals = ['SIGINT', 'SIGTERM'];
    const baseline = signals.map(name => process.listenerCount(name));
    let handled; let calls = 0;
    const remote = createServer(request => {
      request.resume(); calls += 1;
      handled = [signal, signal, ...signals].map(name => process.emit(name));
    });
    remote.listen(0, '127.0.0.1'); await once(remote, 'listening');
    context.after(async () => {
      remote.closeAllConnections(); await new Promise(resolve => remote.close(resolve));
    });
    await run(setup(`http://127.0.0.1:${remote.address().port}`), quiet);
    assert.equal(await run(['check', '--json'], quiet), 1);
    assert.equal(calls, 1);
    assert.deepEqual(handled, [true, true, true, true]);
    assert.deepEqual(signals.map(name => process.listenerCount(name)), baseline);
  });
}

for (const json of [false, true]) {
  for (const signal of ['SIGINT', 'SIGTERM']) {
    test(`${signal} preserves failed cleanup during cancelled startup with ${json ? 'JSON' : 'compact'} output`,
      { timeout: 5000 }, async context => {
      await fixture(context); const remote = await peer(context);
      await run(setup(remote.origin), quiet);
      const signals = ['SIGINT', 'SIGTERM'];
      const baseline = signals.map(name => process.listenerCount(name));
      const closed = Promise.withResolvers();
      const originalListen = Server.prototype.listen;
      const originalClose = Server.prototype.close;
      let closes = 0; let port; let text = '';
      context.mock.method(Server.prototype, 'listen', function(...args) {
        this.once('listening', () => {
          port = this.address().port; process.emit(signal);
        });
        return originalListen.apply(this, args);
      });
      context.mock.method(Server.prototype, 'close', function(callback) {
        if (this === remote.server) return originalClose.call(this, callback);
        closes += 1;
        originalClose.call(this, () => closed.resolve());
        throw new Error('private-close-canary');
      });
      const result = await run(['serve', ...(json ? ['--json'] : [])], {
        ...quiet, stdout(chunk) { text += chunk; },
      }).then(code => ({ code }), error => ({ error }));
      await closed.promise;
      await refused(port);
      assert.equal(closes, 1);
      assert.equal(remote.calls(), 1);
      assert.deepEqual(signals.map(name => process.listenerCount(name)), baseline);
      assert.equal(result.error?.code, 'cleanup_failed');
      assert.equal(result.error?.message, 'Remote cleanup failed.');
      assert.equal(result.code, undefined);
      assert.equal(text, '');
    });
  }
}

async function refused(port) {
  await new Promise((resolve, reject) => {
    const socket = createConnection({ host: '127.0.0.1', port });
    socket.once('connect', () => { socket.destroy(); reject(new Error('listener remained open')); });
    socket.once('error', error => error.code === 'ECONNREFUSED' ? resolve() : reject(error));
  });
}

test('failed replacement removes its private staging directory', async context => {
  const { root, profile } = await fixture(context);
  await mkdir(profile, { recursive: true });
  await assert.rejects(run(setup('http://127.0.0.1'), quiet), /could not write/);
  assert.deepEqual(await readdir(join(root, 'profile')), ['framerize-remote.json']);
  assert.deepEqual(await readdir(profile), []);
});

for (const status of [401, 503]) {
  test(`startup refuses readiness status ${status} and retains the profile`, async context => {
    const { profile } = await fixture(context); const remote = await peer(context, status);
    const servers = process.getActiveResourcesInfo().filter(name => name === 'TCPServerWrap');
    let output = '';
    const code = await run([...setup(remote.origin), '--start', '--json'], {
      ...quiet, stdout(text) { output += text; },
    });
    assert.equal(code, 1); assert.equal(remote.calls(), 1);
    assert.deepEqual(process.getActiveResourcesInfo().filter(name => name === 'TCPServerWrap'), servers);
    const events = output.trim().split('\n').map(JSON.parse);
    assert.equal(events[1].diagnostic.code, status === 401 ? 'remote_auth' : 'upstream_unavailable');
    assert.equal(typeof events[1].diagnostic.next, 'string');
    delete events[1].diagnostic;
    assert.deepEqual(events, [
      { saved: true }, { ready: false, reason: 'upstream_unavailable' },
    ]);
    assert.ok(await readFile(profile));
  });
}

test('atomic saves replace a linked profile without changing its other name', async context => {
  const { root, profile } = await fixture(context);
  await mkdir(join(root, 'profile'));
  const original = join(root, 'original.json');
  await writeFile(original, 'keep me', { mode: 0o666 }); await link(original, profile);
  await Promise.all(Array.from({ length: 4 }, () => run(setup('http://127.0.0.1'), quiet)));
  assert.equal(await readFile(original, 'utf8'), 'keep me');
  assert.equal(JSON.parse(await readFile(profile, 'utf8')).port, 0);
  assert.deepEqual(await readdir(join(root, 'profile')), ['framerize-remote.json']);
  if (process.platform !== 'win32') assert.equal((await stat(profile)).mode & 0o777, 0o600);
  const before = await readFile(profile);
  await assert.rejects(run([...setup('http://127.0.0.1'), '--port', 'bad'], quiet));
  assert.deepEqual(await readFile(profile), before);
});

test('explicit profiles override the environment and the home default is stable', async context => {
  const { root } = await fixture(context);
  process.env.FRAMERIZE_REMOTE_HOME = 'invalid-relative-directory';
  await run([...setup('http://127.0.0.1'), '--config', 'explicit.json'], quiet);
  assert.equal(JSON.parse(await readFile('explicit.json', 'utf8')).remote, 'http://127.0.0.1');
  await assert.rejects(run(setup('http://127.0.0.1'), quiet), /absolute directory/);
  assert.equal(await run(['setup', '--help'], quiet), 0);
  delete process.env.FRAMERIZE_REMOTE_HOME;
  process.env.HOME = root; process.env.USERPROFILE = root;
  await run(setup('http://127.0.0.1'), quiet);
  assert.equal(JSON.parse(await readFile(join(root, '.framerize-remote', 'framerize-remote.json'), 'utf8')).port, 0);
});

for (const linked of [false, true]) {
  test(`saved file references survive a directory change with linked=${linked}`,
    context => verifySavedReferences(context, linked));
}

async function verifySavedReferences(context, linked) {
  const { root, profile } = await fixture(context, linked);
  const working = process.cwd();
  const remote = await peer(context);
  await writeFile('server.token', 'synthetic-server'); await writeFile('local.token', 'synthetic-local');
  await run(['setup', '--remote', remote.origin, '--server-token-file', 'server.token',
    '--local-token-file', 'local.token'], quiet);
  const saved = JSON.parse(await readFile(profile, 'utf8'));
  assert.equal(saved.serverToken.path, join(working, 'server.token'));
  assert.equal(saved.localToken.path, join(working, 'local.token'));
  assert.doesNotMatch(JSON.stringify(saved), /synthetic-(?:local|server)/);
  if (process.platform !== 'win32') assert.equal((await stat(profile)).mode & 0o777, 0o600);
  process.chdir(join(root, 'other'));
  assert.equal(await run(['check'], quiet), 0);
  assert.equal(remote.calls(), 1);
}

async function peer(context, status = 200) {
  let calls = 0;
  const server = createServer((request, response) => {
    calls += 1; request.resume();
    assert.equal(request.url, '/ready');
    assert.equal(request.headers.authorization, 'Bearer synthetic-server');
    response.statusCode = status;
    response.end(JSON.stringify(status === 200 ? ready : {
      ...ready, status: 'unavailable', reason: 'upstream_unavailable',
    }));
  });
  server.listen(0, '127.0.0.1'); await once(server, 'listening');
  context.after(async () => {
    server.closeAllConnections(); await new Promise(resolve => server.close(resolve));
  });
  return { server, origin: `http://127.0.0.1:${server.address().port}`, calls: () => calls };
}

async function fixture(context, linked = false) {
  const temporary = await mkdtemp(join(tmpdir(), 'remote-first-run-'));
  const root = linked ? join(temporary, 'linked') : temporary;
  if (linked) {
    await mkdir(join(temporary, 'actual'));
    await symlink(join(temporary, 'actual'), root, 'junction');
  }
  const previous = process.cwd();
  const keys = ['FRAMERIZE_REMOTE_HOME', 'FIRST_SERVER', 'FIRST_LOCAL', 'HOME', 'USERPROFILE'];
  const saved = keys.map(key => [key, process.env[key]]);
  await mkdir(join(root, 'working')); await mkdir(join(root, 'other'));
  process.chdir(join(root, 'working'));
  process.env.FRAMERIZE_REMOTE_HOME = join(root, 'profile');
  process.env.FIRST_SERVER = 'synthetic-server'; process.env.FIRST_LOCAL = 'synthetic-local';
  context.after(async () => {
    process.chdir(previous);
    for (const [key, value] of saved) {
      if (value === undefined) delete process.env[key]; else process.env[key] = value;
    }
    await rm(temporary, { recursive: true, force: true });
  });
  return { root, profile: join(root, 'profile', 'framerize-remote.json') };
}
