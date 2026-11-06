import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { basename, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const source = fileURLToPath(new URL('../src/', import.meta.url));
const replacements = {
  'clientCli.ts': 'export const offerClientSetup = (...args) => globalThis.__remoteEnvOffer(...args); export const runClientCommand = () => { throw new Error("unexpected client"); };',
  'cmdRun.ts': 'export const startServe = (...args) => globalThis.__remoteEnvServe(...args); export class StartupCleanupError extends Error {} export class StartupReadinessError extends Error {} export const runCheck = () => { throw new Error("unexpected check"); };',
};
const compiled = await build({ stdin: { resolveDir: source,
  contents: "export { run, serve } from './cli.ts'; export { runSetup } from './cmdSetup.ts'; export { loadRunConfig } from './configStore.ts'; export { loadCredential } from './credentials.ts'; export { nodeRunner } from './compatibility/process.ts';" },
  bundle: true, platform: 'node', format: 'esm', write: false,
  alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) },
  plugins: [{ name: 'isolate-client-and-listener', setup(builder) {
    builder.onLoad({ filter: /(clientCli|cmdRun)\.ts$/ }, args =>
      dirname(args.path) === resolve(source)
        ? { contents: replacements[basename(args.path)], loader: 'ts' } : undefined);
  } }],
});
const { run, serve, runSetup, loadRunConfig, loadCredential, nodeRunner } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const insensitive = values => new Proxy(values, { get(target, name) {
  if (typeof name !== 'string') return Reflect.get(target, name);
  const key = Object.keys(target).find(key => key.toLowerCase() === name.toLowerCase());
  return key === undefined ? Reflect.get(target, name) : target[key];
} });

async function fixture(context) {
  const home = await mkdtemp(join(tmpdir(), 'frm-remote-env-caller-'));
  const original = process.env;
  const before = ['SIGINT', 'SIGTERM'].map(name => process.listeners(name));
  const hooks = [globalThis.__remoteEnvOffer, globalThis.__remoteEnvServe];
  context.after(async () => {
    process.env = original;
    [globalThis.__remoteEnvOffer, globalThis.__remoteEnvServe] = hooks;
    await rm(home, { recursive: true, force: true });
    assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => process.listeners(name)), before);
  });
  const values = { HOME: home, TMPDIR: home, frm_remote: 'http://127.0.0.1:9',
    frm_host: '127.0.0.2', frm_port: 'invalid', frm_local_token: 'fixture-local-token',
    pAtH: 'fixture-search-path', Custom_Value: 'fixture-other-value' };
  const env = insensitive(values);
  return { home, path: join(home, 'profile.json'), values, env };
}

test('explicit setup port preserves other case-insensitive lookups', async context => {
  const state = await fixture(context);
  let failure, saved;
  try {
    await runSetup(['--port', '8788'], state.path, state.env);
    saved = JSON.parse(await readFile(state.path, 'utf8'));
  } catch (error) { failure = error.code ?? error.message; }
  await runSetup(['--remote', 'http://127.0.0.1:9', '--port', '8788'], state.path, {});
  assert.equal((await loadRunConfig(state.path, {})).port, 8788);
  await rm(state.home, { recursive: true, force: true });
  assert.equal(failure, undefined);
  assert.equal(saved.remote, 'http://127.0.0.1:9');
  assert.equal(saved.host, '127.0.0.2');
  assert.equal(saved.port, 8788);
});

for (const mode of ['setup', 'serve', 'uppercase control']) {
  test(`${mode} preserves lookups and the spawned environment`, async context => {
    const state = await fixture(context);
    if (mode === 'uppercase control') {
      for (const key of Object.keys(state.values)) {
        const value = state.values[key]; delete state.values[key]; state.values[key.toUpperCase()] = value;
      }
    }
    const argv = ['--remote', 'http://127.0.0.1:9', '--host', '127.0.0.1',
      '--port', '8788', '--local-token-env', 'FRM_LOCAL_TOKEN'];
    if (mode === 'serve') await runSetup(argv, state.path, {});
    process.env = state.env;
    const observed = [], spawned = [];
    let closed = 0;
    const runner = nodeRunner('win32', (file, args, options) => {
      spawned.push({ file, args, env: options.env });
      const child = new EventEmitter();
      child.kill = () => { throw new Error('unexpected child stop'); };
      queueMicrotask(() => child.emit('close', 0));
      return child;
    });
    globalThis.__remoteEnvServe = async () => ({ address: { host: '127.0.0.1', port: 41888 },
      close: async () => { closed++; } });
    globalThis.__remoteEnvOffer = async (path, io, env) => {
      let target, token, failure;
      try {
        const config = await loadRunConfig(path, env);
        target = { host: config.host, port: config.port };
        token = await loadCredential(config.localToken, env);
      } catch (error) { failure = error.code ?? error.message; }
      observed.push({ target, token, failure, search: env.PATH, custom: env.CUSTOM_VALUE });
      assert.equal(await runner.run({ command: 'fixture-client.exe', args: [] }, env), 0);
      return mode === 'serve' ? 'cancelled' : undefined;
    };
    const output = [];
    const io = { stdout: value => output.push(value), stderr: value => output.push(value), ask: async () => 'yes' };
    const result = mode === 'serve' ? await serve(state.path, io, false, undefined, true)
      : await run(['setup', '--config', state.path, ...argv], io);
    assert.equal(result, 0);
    assert.equal(closed, mode === 'serve' ? 1 : 0);
    assert.equal(spawned.length, 1);
    assert.equal(spawned[0].file, 'fixture-client.exe');
    const searchKey = mode === 'uppercase control' ? 'PATH' : 'pAtH';
    const customKey = mode === 'uppercase control' ? 'CUSTOM_VALUE' : 'Custom_Value';
    assert.equal(spawned[0].env[searchKey], 'fixture-search-path');
    assert.equal(spawned[0].env[customKey], 'fixture-other-value');
    await rm(state.home, { recursive: true, force: true });
    assert.deepEqual(observed, [{ target: { host: '127.0.0.1', port: mode === 'serve' ? 41888 : 8788 },
      token: 'fixture-local-token', failure: undefined, search: 'fixture-search-path', custom: 'fixture-other-value' }]);
  });
}
