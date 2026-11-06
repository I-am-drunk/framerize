import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import https from 'node:https';
import net from 'node:net';
import childProcess from 'node:child_process';
import { createHash } from 'node:crypto';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/setup.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
  plugins: [{ name: 'source-boundaries', setup(builder) {
    builder.onResolve({ filter: /^\.\/(backend|account-cli|browser-login)\.js$/ },
      args => ({ path: args.path, external: true }));
    builder.onResolve({ filter: /\/dist\/core\.js$/ }, () => ({
      path: fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)),
    }));
  } }],
});
stop();
assert.ok(Object.keys(compiled.metafile.inputs).every(path => !/castle|backend\.ts|browser-login/.test(path)));
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
console.log(JSON.stringify({ runtime: process.version, bundle: hash(compiled.outputFiles[0].contents),
  inputs: await Promise.all(Object.keys(compiled.metafile.inputs).sort().map(async path =>
    [path, hash(await fs.readFile(path))])) }));
const { runSetupCommand } = await import(`data:text/javascript;base64,${
  Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function bounded(pending) {
  let timer;
  try {
    return await Promise.race([pending, new Promise(resolve => {
      timer = setTimeout(() => resolve({ timedOut: true }), 1000);
    })]);
  } finally { clearTimeout(timer); }
}

async function fixture(context, command, exists) {
  const root = await fs.mkdtemp(join(tmpdir(), 'setup-presence-'));
  const state = { root, controller: new AbortController(), pending: undefined, commands: [],
    output: '', readiness: 0, forbidden: 0, release() {} };
  context.after(async () => {
    state.release();
    state.controller.abort(new Error('fixture cleanup'));
    const outcome = await bounded(state.pending ?? Promise.resolve({}));
    context.mock.restoreAll();
    syncBuiltinESMExports();
    await fs.rm(root, { recursive: true, force: true });
    await assert.rejects(fs.stat(root), { code: 'ENOENT' });
    assert.notEqual(outcome.timedOut, true, 'Owned setup must finish during cleanup');
    context.diagnostic('held presence check released; private state removed');
  });
  state.path = join(root, 'server.json');
  state.bytes = Buffer.from(JSON.stringify({ version: 1, environment: {}, config: {
    backend: 'http', upstream: 'http://127.0.0.1:9', host: '127.0.0.1', port: 8789,
    privateNetworkOnly: false,
  } }));
  if (exists) await fs.writeFile(state.path, state.bytes, { mode: 0o600 });
  state.originalRead = fs.readFile.bind(fs);
  const deny = () => { state.forbidden++; throw new Error('Unexpected external operation'); };
  context.mock.method(globalThis, 'fetch', deny);
  for (const target of [http, https]) {
    for (const name of ['request', 'get', 'createServer']) context.mock.method(target, name, deny);
  }
  for (const name of ['connect', 'createConnection', 'createServer']) context.mock.method(net, name, deny);
  for (const name of ['spawn', 'exec', 'execFile', 'spawnSync', 'execSync', 'execFileSync', 'fork']) {
    context.mock.method(childProcess, name, deny);
  }
  syncBuiltinESMExports();
  state.expectedCommands = command === 'uninstall'
    ? [['systemctl', '--user', 'is-enabled', 'framerize-server.service']] : [];
  state.invoke = () => {
    const args = command === 'up' ? ['up', '--backend', 'http', '--upstream',
      'http://127.0.0.1:9', '--host', '127.0.0.1', '--port', '8789', '--no-lan', '--json']
      : ['service', 'uninstall', '--json'];
    state.pending = runSetupCommand(args, {
      signal: state.controller.signal, platform: 'linux', uid: 501,
      env: { HOME: root, XDG_CONFIG_HOME: join(root, 'xdg'), FRAMERIZE_SERVER_HOME: root },
      program: [process.execPath, join(root, 'synthetic-server.js')],
      stdout: { write(text) { state.output += text; } },
      run: async (program, args) => { state.commands.push([program, ...args]); return { code: 1, stdout: '' }; },
      ready: async () => { state.readiness++; throw new Error('Unexpected readiness'); },
    }).then(value => ({ value }), error => ({ error }));
    return state.pending;
  };
  state.unchanged = async () => {
    if (exists) assert.deepEqual(await state.originalRead(state.path), state.bytes);
    assert.deepEqual(await fs.readdir(root), exists ? ['server.json'] : []);
    assert.deepEqual(state.commands, state.expectedCommands);
    assert.equal(state.output, '');
    assert.equal(state.readiness + state.forbidden, 0);
  };
  return state;
}

for (const command of ['up', 'uninstall']) {
  for (const exists of [true, false]) {
    test(`${command} cancels pending presence check for ${exists ? 'existing' : 'missing'} settings`,
      { timeout: 5000 }, async context => {
        const state = await fixture(context, command, exists);
        const entered = Promise.withResolvers(), released = Promise.withResolvers();
        const originalAccess = fs.access;
        let accesses = 0, completions = 0;
        state.release = () => released.resolve();
        context.mock.method(fs, 'access', async function (path, ...args) {
          assert.equal(String(path), state.path, 'Only private settings may be checked');
          accesses++;
          const result = await originalAccess.call(this, path, ...args)
            .then(() => ({}), error => ({ error }));
          entered.resolve({ entered: true });
          await released.promise;
          completions++;
          if (result.error) throw result.error;
        });
        syncBuiltinESMExports();
        const pending = state.invoke();
        assert.equal((await bounded(Promise.race([entered.promise, pending]))).entered, true);
        const reason = new Error('synthetic settings presence cancellation');
        state.controller.abort(reason);
        const beforeRelease = await bounded(pending);
        await state.unchanged();
        assert.equal(completions, 0, 'The presence operation stays held during observation');
        state.release();
        const settled = await bounded(pending);
        assert.equal(settled.error, reason, 'The original cancellation reason survives');
        await state.unchanged();
        assert.equal(accesses, 1);
        assert.equal(completions, 1);
        assert.equal(getEventListeners(state.controller.signal, 'abort').length, 0);
        context.diagnostic(JSON.stringify({ command, exists, accesses, completions,
          settledBeforeRelease: beforeRelease.error === reason,
          commands: state.commands.length, readiness: state.readiness, forbidden: state.forbidden }));
        assert.equal(beforeRelease.error, reason, 'Cancellation must not wait for the presence check');
      });
  }
}
