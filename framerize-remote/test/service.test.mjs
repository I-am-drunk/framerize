// Cases adapted from T3 Code apps/server/src/cloud/bootService.test.ts at 5a906583 (MIT, T3 Tools Inc.).
import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const source = name => fileURLToPath(new URL(`../src/${name}.ts`, import.meta.url));
const compiled = await build({ stdin: { contents: `export * from ${JSON.stringify(source('serviceControl'))};
  export * from ${JSON.stringify(source('serviceUnits'))};
  export * from ${JSON.stringify(source('diagnostics'))};`, resolveDir: fileURLToPath(new URL('.', import.meta.url)) },
bundle: true, platform: 'node', format: 'esm', write: false });
const service = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function harness(context, platform = 'linux', profile = { remote: 'http://100.64.0.10:8787' }) {
  const root = await mkdtemp(join(tmpdir(), 'remote-service-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  const home = join(root, 'frm home');
  await mkdir(home);
  await writeFile(join(home, 'framerize-remote.json'), JSON.stringify(profile));
  const commands = [];
  const control = { fail: undefined, active: true, linger: 'yes', manager: true, repairs: true };
  const run = async (command, args) => {
    const line = `${command} ${args.join(' ')}`;
    commands.push(line);
    if (line.endsWith(' stop frm.service') || line.includes(' disable --now ')) control.active = false;
    if (/ (re)?start frm\.service$/.test(line) || line.includes(' bootstrap ')) control.active = true;
    if (line.startsWith('loginctl enable-linger') && control.repairs) control.linger = 'yes';
    if (line.startsWith('loginctl show-user')) return { code: control.linger ? 0 : 1, stdout: `${control.linger}\n` };
    const probeFails = (line.includes(' is-active ') || line.startsWith('launchctl print')) && !control.active
      || (line.endsWith(' show-environment') && !control.manager);
    return { code: line === control.fail || probeFails ? 1 : 0, stdout: '' };
  };
  const make = (serviceHome = home) => ({ host: { platform, homeDir: root, uid: 501 }, home: serviceHome,
    program: ['/usr/bin/node', '/opt/frm/bin/frm.js', 'serve'], environmentPath: '/usr/bin:/bin', run });
  return { root, home, commands, control, context: make(), make };
}

test('reads the served FRM_HOME back out of a rendered unit or plist', () => {
  const plan = home => ({ program: ['/usr/bin/node', `${home}/frm.js`, 'serve'], home, logPath: `${home}/frm-service.log` });
  assert.equal(service.serviceHomeOf(service.renderServiceUnit(plan('/home/user/.framerize-remote'))),
    '/home/user/.framerize-remote');
  assert.equal(service.serviceHomeOf(service.renderServiceUnit(plan('/home/user/FRM "Data"/100%\\x'))),
    '/home/user/FRM "Data"/100%\\x');
  assert.equal(service.serviceHomeOf(service.renderServicePlist(plan('/Users/user/a&b <c>'),
    { homeDir: '/Users/user', environmentPath: '/usr/bin' })), '/Users/user/a&b <c>');
  assert.equal(service.serviceHomeOf('[Service]\nExecStart=/x\n'), undefined);
});

test('installs, reports current state, controls and uninstalls on Linux', async context => {
  const { root, home, commands, context: target } = await harness(context);
  assert.equal(await service.installService(target), 'installed');
  const unitPath = join(root, '.config', 'systemd', 'user', 'frm.service');
  const unit = await readFile(unitPath, 'utf8');
  assert.match(unit, /^ExecStart=\/usr\/bin\/node \/opt\/frm\/bin\/frm\.js serve$/m);
  assert.match(unit, /^Environment=FRM_HOME="[^"]*frm home"$/m);
  assert.deepEqual(commands, ['systemctl --user show-environment', 'loginctl show-user 501 --property=Linger --value',
    'systemctl --user daemon-reload', 'systemctl --user enable frm.service', 'systemctl --user restart frm.service']);
  assert.deepEqual(await service.serviceStatus(target), { supported: true, installed: true, running: true,
    enabled: true, current: true, otherHome: false, unitPath, logPath: join(home, 'frm-service.log') });
  for (const action of ['stop', 'start', 'restart']) assert.equal(await service.controlService(target, action), 'done');
  commands.length = 0;
  assert.equal(await service.uninstallService(target), 'removed');
  assert.deepEqual(commands, ['systemctl --user disable --now frm.service', 'systemctl --user daemon-reload']);
  assert.equal((await service.serviceStatus(target)).installed, false);
  assert.equal(await service.uninstallService(target), 'missing');
});

test('restart and uninstall leave a service that serves another FRM_HOME alone', async context => {
  const { root, commands, context: target, make } = await harness(context);
  await service.installService(target);
  commands.length = 0;
  const other = make(join(root, 'other-home'));
  assert.equal(await service.controlService(other, 'restart'), 'skipped');
  assert.equal(await service.uninstallService(other), 'skipped');
  assert.equal((await service.serviceStatus(other)).otherHome, true);
  await assert.rejects(service.installService({ ...other, home: target.home.replace('frm home', 'absent') }),
    { code: 'profile_missing' });
  assert.deepEqual(commands.filter(command => !command.includes(' is-')), []);
});

test('installs, reports current state, and uninstalls on macOS', async context => {
  const { root, commands, context: target } = await harness(context, 'darwin');
  assert.equal(await service.installService(target), 'installed');
  const unitPath = join(root, 'Library', 'LaunchAgents', 'dev.framerize.frm.service.plist');
  const plist = await readFile(unitPath, 'utf8');
  assert.match(plist, /<string>\/opt\/frm\/bin\/frm\.js<\/string>\n {4}<string>serve<\/string>/);
  assert.match(plist, /<key>PATH<\/key>\n {4}<string>\/usr\/bin:\/bin<\/string>/);
  assert.equal((await service.serviceStatus(target)).current, true);
  assert.equal(await service.uninstallService(target), 'removed');
  assert.equal((await service.serviceStatus(target)).installed, false);
  assert.equal(commands.some(command => command.startsWith('systemctl ')), false);
});

test('ignores a bootout for an agent that is not loaded', async context => {
  const { context: target, control } = await harness(context, 'darwin');
  await service.installService(target);
  control.fail = 'launchctl bootout --wait gui/501/dev.framerize.frm.service';
  assert.equal(await service.installService(target), 'updated');
  assert.equal((await service.serviceStatus(target)).current, true);
});

test('a failed activation step reports the step without printing command output', async context => {
  const { context: target, control } = await harness(context);
  control.fail = 'systemctl --user enable frm.service';
  await assert.rejects(service.installService(target), { code: 'service_failed', message: 'enabling the service' });
});

test('the service refuses token variables it could not read after login', async context => {
  const { context: target, commands } = await harness(context, 'linux',
    { remote: 'http://100.64.0.10:8787', serverToken: { kind: 'env', name: 'FRM_SERVER_TOKEN' } });
  await assert.rejects(service.installService(target), { code: 'service_env_token' });
  assert.deepEqual(commands, []);
});

test('unsupported platforms report status without running commands', async context => {
  const { context: target, commands } = await harness(context, 'win32');
  assert.equal((await service.serviceStatus(target)).supported, false);
  await assert.rejects(service.installService(target), { code: 'service_unsupported' });
  assert.deepEqual(commands, []);
});

test('install with lingering already enabled does not try to enable it', async context => {
  const { commands, context: target } = await harness(context);
  assert.equal(await service.installService(target), 'installed');
  assert.equal(commands.some(command => command.startsWith('loginctl enable-linger')), false);
});

test('enables lingering before installing', async context => {
  const { commands, control, context: target } = await harness(context);
  control.linger = 'no';
  assert.equal(await service.installService(target), 'installed');
  assert.deepEqual(commands.slice(0, 5), ['systemctl --user show-environment',
    'loginctl show-user 501 --property=Linger --value', 'loginctl enable-linger --no-ask-password 501',
    'systemctl --user show-environment', 'loginctl show-user 501 --property=Linger --value']);
  assert.equal(commands[5], 'systemctl --user daemon-reload');
});

test('a failed linger repair stops install with an actionable diagnostic and writes no unit', async context => {
  const { root, commands, control, context: target } = await harness(context);
  control.linger = 'no';
  control.repairs = false;
  control.fail = 'loginctl enable-linger --no-ask-password 501';
  await assert.rejects(service.installService(target), error => error.code === 'service_linger'
    && /^\[linger-disabled\] .*sudo loginctl enable-linger/.test(error.message));
  control.fail = undefined;
  await assert.rejects(service.installService(target), { code: 'service_linger' });
  assert.equal(commands.some(command => command.startsWith('systemctl --user daemon-reload')), false);
  await assert.rejects(readFile(join(root, '.config', 'systemd', 'user', 'frm.service')), { code: 'ENOENT' });
});

test('an unreachable user manager or unreadable linger state fails before any repair', async context => {
  for (const [setup, code] of [[control => { control.manager = false; }, 'service_user_manager'],
    [control => { control.linger = ''; }, 'service_linger_unavailable']]) {
    const { commands, control, context: target } = await harness(context);
    setup(control);
    await assert.rejects(service.installService(target), { code });
    assert.equal(commands.some(command => command.startsWith('loginctl enable-linger')), false);
    assert.equal(commands.some(command => command.includes('daemon-reload')), false);
  }
});

test('macOS install runs no systemd or loginctl prerequisite commands', async context => {
  const { commands, context: target } = await harness(context, 'darwin');
  assert.equal(await service.installService(target), 'installed');
  assert.equal(commands.some(command => /^(systemctl|loginctl) /.test(command)), false);
});

test('restoreService removes a new unit, and restores a stopped one without starting it', async context => {
  const { root, commands, control, context: target } = await harness(context);
  const unitPath = join(root, '.config', 'systemd', 'user', 'frm.service');
  const fresh = await service.snapshotService(target);
  await service.installService(target);
  await service.restoreService(target, fresh);
  await assert.rejects(readFile(unitPath), { code: 'ENOENT' });
  await service.installService(target);
  await service.controlService(target, 'stop');
  const stopped = await service.snapshotService(target);
  assert.deepEqual({ enabled: stopped.enabled, running: stopped.running }, { enabled: true, running: false });
  await writeFile(unitPath, 'changed');
  commands.length = 0;
  await service.restoreService(target, stopped);
  assert.equal(await readFile(unitPath, 'utf8'), stopped.unit);
  assert.equal(control.active, false);
  assert.deepEqual(commands, ['systemctl --user stop frm.service', 'systemctl --user daemon-reload',
    'systemctl --user enable frm.service']);
});

for (const operation of ['prerequisites', 'status']) test(`${operation} drains peer probes before reporting cleanup failure`, async t => {
  const { context: target } = await harness(t);
  if (operation === 'status') await service.installService(target);
  const controller = new AbortController(), started = Promise.withResolvers(), release = Promise.withResolvers();
  const failure = new service.ServiceCommandCleanupError(new Error('synthetic cleanup failure'));
  let calls = 0, settled = false;
  target.signal = controller.signal;
  target.run = async () => {
    if (++calls === 1) {
      await started.promise;
      controller.abort();
      throw new service.RemoteDiagnosticError('aborted');
    }
    started.resolve();
    await release.promise;
    throw failure;
  };
  t.after(() => release.resolve());
  const pending = (operation === 'status' ? service.serviceStatus(target)
    : service.requireServicePrerequisites(target)).finally(() => { settled = true; });
  pending.catch(() => {});
  await started.promise;
  await new Promise(setImmediate);
  assert.equal(settled, false, 'a peer probe still owns cleanup work');
  release.resolve();
  await assert.rejects(pending, error => error === failure);
});

for (const operation of ['repair', 'pid']) test(`${operation} preserves cleanup failure during cancellation`, async t => {
  const { context: target } = await harness(t);
  const controller = new AbortController();
  const failure = new service.ServiceCommandCleanupError(new Error('synthetic cleanup failure'));
  target.signal = controller.signal;
  target.run = async (_program, args) => {
    if (operation === 'pid' || args.includes('enable-linger')) { controller.abort(); throw failure; }
    return { code: 0, stdout: args.includes('show-user') ? 'no\n' : '' };
  };
  await assert.rejects(operation === 'pid' ? service.serviceMainPid(target)
    : service.requireServicePrerequisites(target), error => error === failure);
});
