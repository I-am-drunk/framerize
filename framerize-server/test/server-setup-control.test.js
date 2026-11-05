import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { reconcileService, controlService, ensureServiceHost } from '../dist/service-control.js';
import { selectServiceManager, renderServiceUnit } from '../dist/service-units.js';

async function context(t) {
  const root = await mkdtemp(join(tmpdir(), 'server-service-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const plan = { home: join(root, 'settings 100%$'), homeDir: root, program: ['/synthetic/node', '/synthetic/server'],
    environmentPath: '/synthetic/bin', platform: 'linux', uid: 1000 };
  const state = { running: false, enabled: false, linger: false, calls: [] };
  const run = async (command, args) => {
    state.calls.push([command, ...args]);
    if (command === 'loginctl' && args[0] === 'enable-linger') state.linger = true;
    if (command === 'loginctl') return { code: 0, stdout: state.linger ? 'yes' : 'no' };
    if (args.includes('is-active')) return { code: state.running ? 0 : 1, stdout: '' };
    if (args.includes('is-enabled')) return { code: state.enabled ? 0 : 1, stdout: '' };
    if (args.includes('enable')) state.enabled = true;
    if (args.includes('restart')) state.running = true;
    if (args.includes('stop') || args.includes('disable')) state.running = false;
    if (args.includes('disable')) state.enabled = false;
    return { code: 0, stdout: '' };
  };
  return { plan, state, run, manager: selectServiceManager(plan), settings: join(plan.home, 'server.json') };
}

test('service setup is idempotent and rolls back a failed readiness check', async t => {
  const { plan, state, run, manager, settings } = await context(t);
  assert.equal(await reconcileService(plan, '{"old":true}', async () => {}, run), 'installed');
  const unit = await readFile(manager.path, 'utf8');
  const restarts = () => state.calls.filter(args => args.includes('restart')).length;
  assert.equal(await reconcileService(plan, '{"old":true}', async () => {}, run), 'unchanged');
  assert.equal(restarts(), 1);
  await assert.rejects(reconcileService(plan, '{"new":true}', async () => {
    throw new Error('synthetic readiness failure');
  }, run), /synthetic readiness failure/);
  assert.equal(await readFile(settings, 'utf8'), '{"old":true}');
  assert.equal(await readFile(manager.path, 'utf8'), unit);
  assert.equal(state.running, true);
  assert.equal(state.enabled, true);
});

test('failed first setup removes only its new files; uninstall retains saved settings', async t => {
  const { plan, state, run, manager, settings } = await context(t);
  await assert.rejects(reconcileService(plan, '{}', async () => { throw new Error('not ready'); }, run), /not ready/);
  await assert.rejects(readFile(settings), { code: 'ENOENT' });
  await assert.rejects(readFile(manager.path), { code: 'ENOENT' });
  assert.equal(state.running, false);
  assert.equal(state.enabled, false);
  await reconcileService(plan, '{}', async () => {}, run);
  await controlService(plan, 'uninstall', run);
  await assert.rejects(readFile(manager.path), { code: 'ENOENT' });
  assert.equal(await readFile(settings, 'utf8'), '{}');
  assert.equal(state.running, false);
});

test('setup refuses to replace a service owned by another settings directory', async t => {
  const { plan, state, run, manager } = await context(t);
  const original = renderServiceUnit({ ...plan, home: join(plan.homeDir, 'someone-else') });
  await mkdir(dirname(manager.path), { recursive: true });
  await writeFile(manager.path, original);
  await assert.rejects(reconcileService(plan, '{}', async () => {}, run), /another FRAMERIZE_SERVER_HOME/);
  assert.equal(await readFile(manager.path, 'utf8'), original);
  assert.ok(state.calls.every(args => !args.includes('stop') && !args.includes('restart')));
});

test('Linux setup repairs disabled linger and verifies the result', async t => {
  const { plan, state, run } = await context(t);
  await ensureServiceHost(plan, run);
  assert.equal(state.linger, true);
  assert.equal(state.calls.filter(args => args.includes('enable-linger')).length, 1);
  await ensureServiceHost(plan, run);
  assert.equal(state.calls.filter(args => args.includes('enable-linger')).length, 1);
});
