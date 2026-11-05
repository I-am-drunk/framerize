import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { reconcileService, controlService, ensureServiceHost, serviceStatus } from '../dist/service-control.js';
import { selectServiceManager, renderServiceUnit } from '../dist/service-units.js';

async function context(t) {
  const root = await mkdtemp(join(tmpdir(), 'server-service-test-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const plan = { home: join(root, 'settings 100%$'), homeDir: root, program: ['/synthetic/node', '/synthetic/server'],
    environmentPath: '/synthetic/bin', platform: 'linux', uid: 1000 };
  const state = { running: false, enabled: false, linger: false, pid: 1234, calls: [] };
  const run = async (command, args) => {
    state.calls.push([command, ...args]);
    if (command === 'loginctl' && args[0] === 'enable-linger') state.linger = true;
    if (command === 'loginctl') return { code: 0, stdout: state.linger ? 'yes' : 'no' };
    if (args.includes('is-active')) return { code: state.running ? 0 : 1, stdout: '' };
    if (args.includes('show')) return { code: 0, stdout: state.running
      ? `ActiveState=active\nSubState=running\nMainPID=${state.pid}\n`
      : 'ActiveState=inactive\nSubState=dead\nMainPID=0\n' };
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

test('failed setup leaves an enabled service stopped when it was stopped before setup', async t => {
  const { plan, state, run, manager, settings } = await context(t);
  await reconcileService(plan, '{"old":true}', async () => state.pid, run);
  await controlService(plan, 'stop', run);
  assert.equal(state.enabled, true);
  assert.equal(state.running, false);
  const unit = await readFile(manager.path, 'utf8');
  state.calls.length = 0;
  await assert.rejects(reconcileService(plan, '{"new":true}',
    async () => assert.fail('readiness must not run'), run,
    async () => { throw new Error('synthetic occupied port'); }), /synthetic occupied port/);
  assert.equal(state.running, false);
  assert.equal(state.enabled, true);
  assert.ok(!state.calls.some(args => args.includes('restart')));
  assert.equal(await readFile(settings, 'utf8'), '{"old":true}');
  assert.equal(await readFile(manager.path, 'utf8'), unit);
});

test('readiness from another process cannot validate a newly installed service', async t => {
  const { plan, state, run, manager, settings } = await context(t);
  await assert.rejects(reconcileService(plan, '{}', async () => state.pid + 1, run), /another process/);
  await assert.rejects(readFile(settings), { code: 'ENOENT' });
  await assert.rejects(readFile(manager.path), { code: 'ENOENT' });
  assert.equal(state.running, false);
});

test('a process that exits during readiness cannot validate a service', async t => {
  const { plan, state, run } = await context(t);
  await assert.rejects(reconcileService(plan, '{}', async () => {
    state.running = false;
    return state.pid;
  }, run), /not running/);
});

test('unchanged setup still rejects readiness from another process', async t => {
  const { plan, state, run } = await context(t);
  await reconcileService(plan, '{}', async () => state.pid, run);
  await assert.rejects(reconcileService(plan, '{}', async () => state.pid + 1, run), /another process/);
  assert.equal(state.running, true);
  assert.equal(state.calls.filter(args => args.includes('restart')).length, 1);
});

test('preflight runs after stopping the old service and preserves it on failure', async t => {
  const { plan, state, run, settings } = await context(t);
  await reconcileService(plan, '{"old":true}', async () => state.pid, run);
  await assert.rejects(reconcileService(plan, '{"new":true}', async () => state.pid, run, async () => {
    assert.equal(state.running, false);
    assert.equal(await readFile(settings, 'utf8'), '{"old":true}');
    throw new Error('port is already in use');
  }), /already in use/);
  assert.equal(state.running, true);
  assert.equal(await readFile(settings, 'utf8'), '{"old":true}');
});

test('failed rollback retains the original readiness failure', async t => {
  const { plan, run } = await context(t);
  let restoring = false;
  await assert.rejects(reconcileService(plan, '{}', async () => {
    restoring = true;
    throw new Error('synthetic readiness failure');
  }, (command, args) => restoring && args.includes('daemon-reload')
    ? Promise.resolve({ code: 1, stdout: '' }) : run(command, args)), error => {
    assert.match(error.message, /synthetic readiness failure/);
    assert.match(error.message, /Rollback also failed: .*daemon-reload/);
    assert.ok(error.cause instanceof AggregateError);
    return true;
  });
});

for (const [platform, output] of [
  ['linux', 'ActiveState=activating\nSubState=auto-restart\nMainPID=1234\n'],
  ['linux', 'ActiveState=active\nSubState=exited\nMainPID=1234\n'],
  ['linux', 'ActiveState=active\nSubState=running\nMainPID=0\n'],
  ['darwin', 'state = waiting\n'],
  ['darwin', 'state = running\npid = 0\n'],
]) {
  test(`service status rejects inactive process facts: ${platform} ${output.trim()}`, async t => {
    const { plan } = await context(t);
    const selected = { ...plan, platform };
    const manager = selectServiceManager(selected);
    await mkdir(dirname(manager.path), { recursive: true });
    await writeFile(manager.path, manager.contents);
    const state = await serviceStatus(selected, async () => ({ code: 0, stdout: output }));
    assert.equal(state.running, false);
  });
}

test('launchd status requires a running PID', async t => {
  const { plan } = await context(t);
  const selected = { ...plan, platform: 'darwin' };
  const manager = selectServiceManager(selected);
  await mkdir(dirname(manager.path), { recursive: true });
  await writeFile(manager.path, manager.contents);
  const state = await serviceStatus(selected, async () => ({ code: 0, stdout: 'state = running\npid = 1234\n' }));
  assert.equal(state.running, true);
  assert.equal(state.pid, 1234);
});

test('stop reaches an installed service while it has no running PID', async t => {
  const { plan, state, run, manager } = await context(t);
  await mkdir(dirname(manager.path), { recursive: true });
  await writeFile(manager.path, manager.contents);
  state.enabled = true;
  await controlService(plan, 'stop', run);
  assert.ok(state.calls.some(args => args.includes('stop')));
  assert.equal(state.enabled, true);
});

test('start replaces a loaded launchd job that has no running PID', async t => {
  const { plan } = await context(t);
  const selected = { ...plan, platform: 'darwin' };
  const manager = selectServiceManager(selected);
  await mkdir(dirname(manager.path), { recursive: true });
  await writeFile(manager.path, manager.contents);
  const calls = [];
  await controlService(selected, 'start', async (_command, args) => {
    calls.push(args[0]);
    return { code: 0, stdout: 'state = waiting\n' };
  });
  assert.deepEqual(calls, ['print', 'bootout', 'bootstrap']);
});

test('uninstall removes an unloaded launchd job without bootout', async t => {
  const { plan } = await context(t);
  const selected = { ...plan, platform: 'darwin' };
  const manager = selectServiceManager(selected);
  await mkdir(dirname(manager.path), { recursive: true });
  await writeFile(manager.path, manager.contents);
  const calls = [];
  await controlService(selected, 'uninstall', async (_command, args) => {
    calls.push(args[0]);
    return { code: 1, stdout: '' };
  });
  assert.deepEqual(calls, ['print']);
  await assert.rejects(readFile(manager.path), { code: 'ENOENT' });
});

test('failed setup restores an enabled service that was automatically restarting', async t => {
  const { plan, state, run, manager, settings } = await context(t);
  await mkdir(dirname(manager.path), { recursive: true });
  await writeFile(manager.path, manager.contents);
  await mkdir(plan.home, { recursive: true });
  await writeFile(settings, '{"old":true}');
  state.enabled = true;
  const restarting = async (command, args) => {
    const result = await run(command, args);
    return args.includes('show') && !state.running
      ? { code: 0, stdout: 'ActiveState=activating\nSubState=auto-restart\nMainPID=0\n' } : result;
  };
  await assert.rejects(reconcileService(plan, '{"new":true}', async () => state.pid, restarting,
    async () => { throw new Error('port is already in use'); }), /already in use/);
  assert.equal(state.running, true);
  assert.equal(await readFile(settings, 'utf8'), '{"old":true}');
});
