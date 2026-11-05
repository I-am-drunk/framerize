import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import fs from 'node:fs';
import { mkdtemp, rm } from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { basename, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const fixtures = {
  cmdSetup: `export async function runSetup() { return 'Saved'; }`,
  cmdUp: `export async function setUp() { return { service: 'unavailable' }; }
    export function formatUp() { return 'Ready'; }`,
  serviceCli: `export function serviceContext() { return {}; }
    export function runService() { throw new Error('Unexpected service command'); }`,
  cmdRun: `export const closed = [];
    export class StartupCleanupError extends Error {}
    export class StartupReadinessError extends Error {}
    export function runCheck() { throw new Error('Unexpected readiness command'); }
    export async function startServe() { return { address: { host: '127.0.0.1', port: 8788 },
      close: async () => { closed.push(true); } }; }`,
};
const built = await build({ stdin: { resolveDir: fileURLToPath(new URL('../', import.meta.url)),
  contents: `export { runFrm } from './src/frmCli.ts';
    export { runClient } from './src/compatibility/command.ts';
    export { nodeRunner } from './src/compatibility/process.ts';
    export { offerClientSetup } from './src/clientCli.ts';
    export { commandFailure, ClientCleanupError } from './src/diagnostics.ts';
    export { closed } from './src/cmdRun.ts';` },
  bundle: true, platform: 'node', format: 'esm', write: false,
  plugins: [{ name: 'synthetic-service-boundaries', setup(plugin) {
    plugin.onLoad({ filter: /[\\/](cmdUp|cmdSetup|cmdRun|serviceCli)\.ts$/ },
      args => ({ contents: fixtures[basename(args.path, '.ts')], loader: 'js' }));
  } }],
});
const { runFrm, runClient, nodeRunner, offerClientSetup, commandFailure, ClientCleanupError, closed } =
  await import(`data:text/javascript;base64,${Buffer.from(built.outputFiles[0].contents).toString('base64')}`);
const failure = () => new ClientCleanupError([Object.assign(new Error('Synthetic permission denial'), { code: 'EPERM' })]);
const handlers = () => ['SIGINT', 'SIGTERM'].map(signal => process.listeners(signal));

test('process-group signal failures keep a distinct cleanup diagnostic', async context => {
  context.mock.method(fs, 'openSync', () => { throw new Error('No controlling terminal'); });
  syncBuiltinESMExports();
  context.after(() => { context.mock.restoreAll(); syncBuiltinESMExports(); });
  context.mock.timers.enable({ apis: ['setTimeout'] });
  const signals = [];
  context.mock.method(process, 'kill', (pid, signal) => {
    signals.push([pid, signal]); throw Object.assign(new Error('Denied'), { code: 'EPERM' });
  });
  const child = Object.assign(new EventEmitter(), { pid: 123456 });
  const controller = new AbortController();
  const pending = nodeRunner('linux', () => child).run({ command: 'synthetic', args: [], display: 'synthetic' },
    {}, controller.signal);
  controller.abort(); child.emit('close', 0);
  context.mock.timers.tick(500);
  await assert.rejects(pending, error => error instanceof ClientCleanupError && error.cause.errors.length === 2);
  assert.deepEqual(signals, [[-123456, 'SIGTERM'], [-123456, 'SIGKILL']]);
  assert.match(commandFailure(failure(), false), /^The client command could not stop all of its processes\./);
  assert.equal(JSON.parse(commandFailure(failure(), true)).code, 'client_cleanup_failed');
});

function dependencies(error, stop) {
  return { platform: 'linux', env: { PATH: '/synthetic/bin' }, home: '/synthetic/home',
    stateDir: '/synthetic/state', target: { origin: 'http://127.0.0.1:8788' },
    probe: () => false, prompt: async () => 'install', io: { stdout() {}, stderr() {} },
    runner: { async run() { stop(); throw error; }, async capture() { assert.fail('Unexpected version lookup'); } } };
}

test('standalone installation retains a cleanup failure after cancellation', async () => {
  const error = failure(), before = handlers();
  await assert.rejects(runClient(['claude', '--install', '--no-input'],
    dependencies(error, () => process.emit('SIGINT'))), found => found === error);
  assert.deepEqual(handlers(), before);
});

test('the optional offer retains a cleanup failure after cancellation', async () => {
  const error = failure(), controller = new AbortController();
  const deps = dependencies(error, () => controller.abort());
  await assert.rejects(offerClientSetup('/synthetic/profile', { ...deps.io, ask: async () => 'yes' }, {},
    { ...deps, signal: controller.signal }), found => found === error);
});

for (const argv of [[], ['up'], ['setup'], ['setup', '--start']]) {
  test(`${argv.join(' ') || 'bare frm'} preserves the offer's cleanup failure`, async context => {
    const root = await mkdtemp(join(tmpdir(), 'client-cleanup-diagnostic-'));
    const saved = { FRM_HOME: process.env.FRM_HOME, FRM_REMOTE: process.env.FRM_REMOTE };
    context.after(async () => {
      for (const [name, value] of Object.entries(saved)) {
        if (value === undefined) delete process.env[name]; else process.env[name] = value;
      }
      await rm(root, { recursive: true, force: true });
    });
    process.env.FRM_HOME = root; delete process.env.FRM_REMOTE;
    const error = failure(), before = handlers(); closed.length = 0;
    const io = { stdout() {}, stderr() {}, async ask(_question, signal) {
      assert.ok(signal instanceof AbortSignal); process.emit('SIGINT'); throw error;
    } };
    await assert.rejects(runFrm(argv, io, { program: ['synthetic'] }), found => found === error);
    assert.deepEqual(handlers(), before);
    assert.equal(closed.length, argv.includes('--start') ? 1 : 0);
  });
}
