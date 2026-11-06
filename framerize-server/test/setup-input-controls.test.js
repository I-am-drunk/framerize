import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs/promises';
import cp from 'node:child_process';
import net from 'node:net';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/setup.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
  plugins: [{ name: 'source-boundaries', setup(builder) {
    builder.onResolve({ filter: /^\.\/(backend|account-cli|browser-login)\.js$/ },
      args => ({ path: args.path, external: true }));
    builder.onResolve({ filter: /\/dist\/core\.js$/ }, () => ({
      path: fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)),
    }));
  } }],
});
stop();
const { runSetupCommand } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const settings = port => JSON.stringify({ version: 1, environment: {}, config: {
  backend: 'http', upstream: 'http://127.0.0.1:9', host: '127.0.0.1',
  port, privateNetworkOnly: false,
} });

async function within(pending, phase) {
  let timer;
  try { return await Promise.race([pending, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error('Timed out during ' + phase)), 1500);
  })]); } finally { clearTimeout(timer); }
}

async function fixture(context) {
  const root = await fs.mkdtemp(join(tmpdir(), 'setup-input-control-'));
  const home = join(root, 'home');
  const state = { root, home, path: join(home, 'server.json'),
    controller: new AbortController(), accesses: [], handles: [], commands: [],
    readiness: [], output: '', guards: 0, release() {} };
  context.after(async () => {
    state.release();
    state.controller.abort(new Error('Fixture cleanup'));
    if (state.pending) await within(state.pending.catch(() => {}), 'cleanup');
    if (state.accessPending) await state.accessPending.catch(() => {});
    for (const handle of state.handles) if (handle.fd >= 0) await handle.close();
    context.mock.restoreAll();
    syncBuiltinESMExports();
    await fs.rm(root, { recursive: true, force: true });
    await assert.rejects(fs.access(root), { code: 'ENOENT' });
  });
  await fs.mkdir(home, { mode: 0o700 });
  const access = fs.access.bind(fs), open = fs.open.bind(fs);
  context.mock.method(fs, 'access', (path, ...args) => {
    state.accesses.push(String(path));
    return state.access ? state.access(path, ...args) : access(path, ...args);
  });
  context.mock.method(fs, 'open', async (...args) => {
    const handle = await open(...args);
    state.handles.push(handle);
    return handle;
  });
  const deny = () => { state.guards++; throw new Error('Unexpected process or network'); };
  context.mock.method(cp, 'spawn', deny);
  context.mock.method(cp, 'execFile', deny);
  context.mock.method(net, 'createServer', deny);
  context.mock.method(globalThis, 'fetch', deny);
  syncBuiltinESMExports();
  state.invoke = (argv, withSignal = true) => {
    state.pending = runSetupCommand([...argv, '--json'], {
      ...(withSignal ? { signal: state.controller.signal } : {}),
      env: { HOME: root, XDG_CONFIG_HOME: join(root, 'xdg'), FRAMERIZE_SERVER_HOME: home },
      program: [process.execPath, join(root, 'unused.js')], platform: 'linux', uid: 501,
      stdout: { write(text) { state.output += text; } },
      ready: async (origin, token, signal) => {
        state.readiness.push({ origin, token, signal }); return 123;
      },
      run: async (command, args) => {
        state.commands.push([command, ...args]);
        if (state.run) return state.run(command, args);
        assert.equal(command, 'systemctl');
        assert.equal(args[1], 'is-enabled', 'No service mutation');
        return { code: 1, stdout: '' };
      },
    });
    return state.pending;
  };
  state.assertClean = () => {
    assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
    assert.ok(state.handles.every(handle => handle.fd === -1));
    assert.equal(state.guards, 0);
  };
  return state;
}

test('pair honors an existing explicit config path without a signal', async context => {
  const state = await fixture(context);
  const alternate = join(state.root, 'alternate.json');
  const bytes = settings(9484), decoy = settings(9485);
  await fs.writeFile(alternate, bytes);
  await fs.writeFile(state.path, decoy);
  assert.equal(await state.invoke(['pair', '--config', alternate], false), 0);
  assert.deepEqual(state.accesses, [alternate]);
  assert.deepEqual(state.readiness, [{ origin: 'http://127.0.0.1:9484', token: '', signal: undefined }]);
  assert.equal(JSON.parse(state.output).origin, 'http://127.0.0.1:9484');
  assert.equal(await fs.readFile(alternate, 'utf8'), bytes);
  assert.equal(await fs.readFile(state.path, 'utf8'), decoy);
  assert.deepEqual(state.commands, []);
  state.assertClean();
});

test('cancellation preserves its reason before a late access denial', async context => {
  const state = await fixture(context), bytes = settings(9489);
  await fs.writeFile(state.path, bytes);
  const entered = Promise.withResolvers(), gate = Promise.withResolvers();
  const reason = new Error('Synthetic cancellation');
  const denied = Object.assign(new Error('Late access denial'), { code: 'EACCES' });
  state.release = () => gate.resolve();
  state.access = () => {
    state.accessPending = (async () => {
      entered.resolve();
      await gate.promise;
      throw denied;
    })();
    return state.accessPending;
  };
  const pending = state.invoke(['pair']).then(value => ({ value }), error => ({ error }));
  await within(entered.promise, 'presence entry');
  state.controller.abort(reason);
  assert.equal((await within(pending, 'cancellation before release')).error, reason);
  state.release();
  await state.accessPending.catch(() => {});
  assert.deepEqual(state.accesses, [state.path]);
  assert.equal(await fs.readFile(state.path, 'utf8'), bytes);
  assert.equal(state.output, '');
  assert.deepEqual(state.commands, []);
  assert.deepEqual(state.readiness, []);
  state.assertClean();
});

for (const withSignal of [false, true]) {
  test('up reads accounts normally ' + (withSignal ? 'with' : 'without') + ' a signal', async context => {
    const state = await fixture(context), path = join(state.root, 'accounts.json');
    const bytes = '[]\n', stop = new Error('Stop before any service mutation');
    await fs.writeFile(path, bytes, { mode: 0o600 });
    state.run = async (command, args) => {
      assert.equal(command, 'systemctl');
      assert.deepEqual(args, ['--user', 'show-environment']);
      throw stop;
    };
    await assert.rejects(state.invoke(['up', '--backend', 'http', '--upstream',
      'http://127.0.0.1:9', '--accounts-file', path, '--no-lan'], withSignal),
    error => error === stop);
    assert.equal(state.handles.length, 1);
    assert.equal(await fs.readFile(path, 'utf8'), bytes);
    assert.deepEqual(await fs.readdir(state.home), []);
    assert.equal(state.commands.length, 1);
    assert.deepEqual(state.readiness, []);
    assert.equal(state.output, '');
    state.assertClean();
  });
}

for (const argv of [['up'], ['service', 'uninstall']]) {
  test(argv.join(' ') + ' preserves non-ENOENT access errors', async context => {
    const state = await fixture(context);
    const error = Object.assign(new Error('Synthetic access denial'), { code: 'EACCES' });
    state.access = async () => { throw error; };
    await assert.rejects(state.invoke(argv), actual => actual === error);
    assert.deepEqual(state.accesses, [state.path]);
    assert.equal(state.commands.length, argv[0] === 'up' ? 0 : 1);
    assert.equal(state.output, '');
    assert.deepEqual(state.readiness, []);
    assert.deepEqual(await fs.readdir(state.home), []);
    state.assertClean();
  });
}

for (const exists of [false, true]) {
  test('uninstall preserves ' + (exists ? 'existing' : 'missing') + ' settings', async context => {
    const state = await fixture(context), bytes = settings(9488);
    if (exists) await fs.writeFile(state.path, bytes);
    assert.equal(await state.invoke(['service', 'uninstall']), 0);
    assert.deepEqual(state.accesses, [state.path]);
    assert.deepEqual(JSON.parse(state.output), { service: 'missing' });
    assert.equal(state.commands.length, 2);
    assert.ok(state.commands.every(command => command[2] === 'is-enabled'));
    if (exists) assert.equal(await fs.readFile(state.path, 'utf8'), bytes);
    else assert.deepEqual(await fs.readdir(state.home), []);
    assert.deepEqual(state.readiness, []);
    state.assertClean();
  });
}

test('pair uses first-use settings when the default file is missing', async context => {
  const state = await fixture(context);
  assert.equal(await state.invoke(['pair', '--host', '127.0.0.1', '--port', '9486']), 0);
  assert.deepEqual(state.accesses, [state.path]);
  assert.equal(state.readiness[0].origin, 'http://127.0.0.1:9486');
  assert.equal(state.readiness[0].signal, state.controller.signal);
  assert.deepEqual(await fs.readdir(state.home), []);
  assert.deepEqual(state.commands, []);
  state.assertClean();
});

test('a missing explicit config remains an error instead of using the default', async context => {
  const state = await fixture(context);
  const alternate = join(state.root, 'missing.json'), bytes = settings(9487);
  await fs.writeFile(state.path, bytes);
  await assert.rejects(state.invoke(['pair', '--config', alternate]), {
    message: 'Server settings are unavailable or invalid',
  });
  assert.deepEqual(state.accesses, [alternate]);
  assert.equal(await fs.readFile(state.path, 'utf8'), bytes);
  assert.equal(state.output, '');
  assert.deepEqual(state.readiness, []);
  assert.deepEqual(state.commands, []);
  state.assertClean();
});
