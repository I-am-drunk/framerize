import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

async function sourceModule(name) {
  const compiled = await build({
    entryPoints: [fileURLToPath(new URL(`../src/${name}.ts`, import.meta.url))],
    bundle: true, platform: 'node', format: 'esm', write: false,
  });
  return import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);
}
const { controlService } = await sourceModule('service-control');
const { selectServiceManager } = await sourceModule('service-units');

async function fixture(context, platform, running, otherHome = false) {
  const root = await mkdtemp(join(tmpdir(), 'service-start-idempotent-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  const plan = { home: join(root, 'server'), homeDir: root,
    xdgConfigHome: join(root, 'xdg'), platform, uid: 1000,
    program: ['/synthetic/node', '/synthetic/server'], environmentPath: '/synthetic/bin' };
  const manager = selectServiceManager(plan);
  const unit = otherHome ? selectServiceManager({ ...plan, home: join(root, 'other') }).contents
    : manager.contents;
  const settingsPath = join(plan.home, 'server.json');
  const settings = '{"synthetic":"unchanged"}\n';
  await mkdir(dirname(manager.path), { recursive: true });
  await mkdir(plan.home, { recursive: true });
  await writeFile(manager.path, unit, { mode: 0o600 });
  await writeFile(settingsPath, settings, { mode: 0o600 });
  const calls = [], mutations = [];
  const run = async (command, args) => {
    calls.push([command, ...args]);
    assert.equal(command, platform === 'linux' ? 'systemctl' : 'launchctl');
    const action = args[platform === 'linux' ? 1 : 0];
    if (action === 'show') return { code: 0, stdout: running
      ? 'ActiveState=active\nSubState=running\nMainPID=1234\n'
      : 'ActiveState=inactive\nSubState=dead\nMainPID=0\n' };
    if (action === 'print') return { code: 0, stdout: running
      ? 'state = running\npid = 1234\n' : 'state = waiting\n' };
    if (action === 'is-enabled') return { code: 0, stdout: 'enabled\n' };
    assert.ok(['daemon-reload', 'enable', 'restart', 'bootout', 'bootstrap'].includes(action));
    mutations.push(action);
    return { code: 0, stdout: '' };
  };
  const unchanged = async () => {
    assert.equal(await readFile(manager.path, 'utf8'), unit);
    assert.equal(await readFile(settingsPath, 'utf8'), settings);
  };
  return { plan, run, calls, mutations, unchanged };
}

for (const platform of ['linux', 'darwin']) {
  test(`start leaves a running ${platform} service untouched`, { timeout: 5000 }, async context => {
    const fixtureState = await fixture(context, platform, true);
    assert.equal(await controlService(fixtureState.plan, 'start', fixtureState.run), 'done');
    context.diagnostic(JSON.stringify({ platform, calls: fixtureState.calls }));
    await fixtureState.unchanged();
    assert.deepEqual(fixtureState.mutations, [], 'Starting a running service must not restart it');
  });

  test(`start still recovers a stopped ${platform} service`, { timeout: 5000 }, async context => {
    const fixtureState = await fixture(context, platform, false);
    assert.equal(await controlService(fixtureState.plan, 'start', fixtureState.run), 'done');
    await fixtureState.unchanged();
    assert.deepEqual(fixtureState.mutations, platform === 'linux'
      ? ['daemon-reload', 'enable', 'restart'] : ['bootout', 'bootstrap']);
  });

  test(`explicit restart still restarts a running ${platform} service`, { timeout: 5000 }, async context => {
    const fixtureState = await fixture(context, platform, true);
    assert.equal(await controlService(fixtureState.plan, 'restart', fixtureState.run), 'done');
    await fixtureState.unchanged();
    assert.deepEqual(fixtureState.mutations, platform === 'linux'
      ? ['restart'] : ['bootout', 'bootstrap']);
  });

  test(`start refuses another home's running ${platform} service`, { timeout: 5000 }, async context => {
    const fixtureState = await fixture(context, platform, true, true);
    await assert.rejects(controlService(fixtureState.plan, 'start', fixtureState.run),
      /belongs to another FRAMERIZE_SERVER_HOME/);
    await fixtureState.unchanged();
    assert.deepEqual(fixtureState.mutations, []);
  });
}
