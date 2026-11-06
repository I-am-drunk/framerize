import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const clients = `export const offers = [];
  export const CLIENT_HELP = '';
  const denied = () => { throw Error('Unexpected client command'); };
  export const nodeRunner = () => ({ run: denied, capture: denied });
  export const parseClientOptions = denied;
  export const runClient = denied;
  export async function offerClients(deps) { offers.push(deps.target); }`;
const runtime = `export class StartupCleanupError extends Error {}
  export class StartupReadinessError extends Error {}
  export function runCheck() { throw Error('Unexpected readiness check'); }
  export function startServe() { throw Error('Unexpected listener'); }`;
const built = await build({ stdin: {
  resolveDir: fileURLToPath(new URL('../', import.meta.url)),
  contents: `export { run } from './src/cli.ts';
    export { loadRunConfig } from './src/configStore.ts';
    export { clientDependencies } from './src/clientCli.ts';
    export { offers } from './src/compatibility/index.ts';` },
  bundle: true, platform: 'node', format: 'esm', write: false,
  plugins: [{ name: 'synthetic-command-boundaries', setup(plugin) {
    plugin.onLoad({ filter: /[\\/]compatibility[\\/]index\.ts$/ }, () => ({ contents: clients }));
    plugin.onLoad({ filter: /[\\/]cmdRun\.ts$/ }, () => ({ contents: runtime }));
  } }],
});
const { run, loadRunConfig, clientDependencies, offers } = await import(
  `data:text/javascript;base64,${Buffer.from(built.outputFiles[0].contents).toString('base64')}`);

test('setup offer uses its saved token while later client calls use environment overrides', async context => {
  const root = await mkdtemp(join(tmpdir(), 'setup-offer-selection-'));
  const relevant = key => /^(FRM_|FRAMERIZE_REMOTE_)/.test(key) || key === 'SETUP_SELECTED_TOKEN';
  const original = Object.fromEntries(Object.entries(process.env).filter(([key]) => relevant(key)));
  const signals = ['SIGINT', 'SIGTERM'].map(name => process.listeners(name));
  context.after(async () => {
    for (const key of Object.keys(process.env).filter(relevant)) delete process.env[key];
    Object.assign(process.env, original);
    await rm(root, { recursive: true, force: true });
  });
  for (const key of Object.keys(process.env).filter(relevant)) delete process.env[key];
  Object.assign(process.env, { FRM_HOME: root, FRM_REMOTE: 'http://127.0.0.1:5678',
    FRM_LOCAL_TOKEN: 'synthetic-ambient', SETUP_SELECTED_TOKEN: 'synthetic-selected' });
  const path = join(root, 'profile.json');
  const output = [];
  const io = { stdout: text => output.push(text), stderr: text => output.push(text), ask: async () => 'yes' };
  offers.length = 0;
  assert.equal(await run(['setup', '--config', path, '--remote', 'http://127.0.0.1:1234',
    '--local-token-env', 'SETUP_SELECTED_TOKEN', '--port', '4321'], io), 0);
  assert.deepEqual(offers, [{ origin: 'http://127.0.0.1:4321', localToken: 'synthetic-selected' }]);
  const before = await readFile(path, 'utf8');
  const later = await clientDependencies(path, io, process.env);
  assert.equal(later.target.localToken, 'synthetic-ambient');
  assert.equal((await loadRunConfig(path, process.env)).remote, process.env.FRM_REMOTE);
  assert.equal(await readFile(path, 'utf8'), before);
  assert.equal(process.env.FRM_LOCAL_TOKEN, 'synthetic-ambient');
  assert.match(output.join(''), /Start a client with frm client/);
  assert.deepEqual(['SIGINT', 'SIGTERM'].map(name => process.listeners(name)), signals);
});

test('setup keeps ambient credential fallback and the actual bound port', async context => {
  const root = await mkdtemp(join(tmpdir(), 'setup-credential-fallback-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  const path = join(root, 'profile.json');
  const saved = JSON.stringify({ remote: 'http://127.0.0.1:1234', port: 0 });
  await writeFile(path, saved);
  const env = { FRM_REMOTE: 'http://127.0.0.1:5678', FRM_PORT: '4321',
    FRM_SERVER_TOKEN: 'synthetic-server', FRM_LOCAL_TOKEN: 'synthetic-local' };
  const config = await loadRunConfig(path, env, true);
  assert.equal(config.remote, 'http://127.0.0.1:1234');
  assert.deepEqual(config.serverToken, { kind: 'env', name: 'FRM_SERVER_TOKEN' });
  assert.deepEqual(config.localToken, { kind: 'env', name: 'FRM_LOCAL_TOKEN' });
  const io = { stdout() {}, stderr() {} };
  const deps = await clientDependencies(path, io, env, undefined, true);
  assert.deepEqual(deps.target, { origin: 'http://127.0.0.1:4321', localToken: 'synthetic-local' });
  assert.equal(await readFile(path, 'utf8'), saved);
});

test('setup follow-up requires the saved profile while ordinary environment-only use remains valid', async context => {
  const root = await mkdtemp(join(tmpdir(), 'setup-profile-missing-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  const path = join(root, 'missing.json');
  const env = { FRM_REMOTE: 'http://127.0.0.1:1234' };
  await assert.rejects(loadRunConfig(path, env, true), { code: 'profile_missing' });
  assert.equal((await loadRunConfig(path, env)).remote, env.FRM_REMOTE);
  await assert.rejects(readFile(path), { code: 'ENOENT' });
});
