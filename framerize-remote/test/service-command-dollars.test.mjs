import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/serviceUnits.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const service = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const plan = program => ({ program, home: '/synthetic/profile', logPath: '/synthetic/profile/service.log' });
const commandLine = program => service.renderServiceUnit(plan(program)).split('\n').find(line => line.startsWith('ExecStart='));

const cases = [
  ['braced variable in a literal package path', ['/usr/bin/node', '/opt/${FRM_TEST_ROOT}/frm.js', 'serve'],
    'ExecStart=/usr/bin/node /opt/$${FRM_TEST_ROOT}/frm.js serve'],
  ['standalone variable-shaped argument', ['/usr/bin/node', '/opt/frm.js', '$USER'],
    'ExecStart=/usr/bin/node /opt/frm.js $$USER'],
  ['dollars in a quoted path', ['/usr/bin/node', '/opt/cash ${FRM_TEST_ROOT}/frm.js', 'serve'],
    'ExecStart=/usr/bin/node "/opt/cash $${FRM_TEST_ROOT}/frm.js" serve'],
  ['literal doubled dollars', ['/usr/bin/node', '/opt/cash$$/frm.js', 'serve'],
    'ExecStart=/usr/bin/node /opt/cash$$$$/frm.js serve'],
];

for (const [name, program, expected] of cases) {
  test(`systemd preserves ${name}`, () => assert.equal(commandLine(program), expected));
}

test('ordinary commands retain quoting and percent-specifier escaping', () => {
  assert.equal(commandLine(['/usr/bin/node', '/opt/frm.js', 'serve']),
    'ExecStart=/usr/bin/node /opt/frm.js serve');
  assert.equal(commandLine(['/usr/bin/node', '/opt/100% "quoted"/frm.js', 'serve']),
    'ExecStart=/usr/bin/node "/opt/100%% \\"quoted\\"/frm.js" serve');
});

test('environment and log paths do not receive command-only dollar escaping', () => {
  const home = '/synthetic/profile ${HOME} 100%';
  const unit = service.renderServiceUnit({
    program: ['/usr/bin/node', '/opt/frm.js', 'serve'], home, logPath: `${home}/service.log`,
  });
  assert.ok(unit.split('\n').includes('Environment=FRM_HOME="/synthetic/profile ${HOME} 100%%"'));
  assert.ok(unit.split('\n').includes('StandardOutput=append:/synthetic/profile ${HOME} 100%%/service.log'));
  assert.ok(unit.split('\n').includes('StandardError=append:/synthetic/profile ${HOME} 100%%/service.log'));
  assert.equal(service.serviceHomeOf(unit), home);
});

test('launchd retains literal dollar arguments without systemd escaping', () => {
  const input = { program: ['/usr/bin/node', '/opt/${HOME}/cash$$.js', '$USER'],
    home: '/synthetic/${PROFILE}', logPath: '/synthetic/${PROFILE}/service.log' };
  const plist = service.renderServicePlist(input, { homeDir: '/synthetic', environmentPath: '/usr/bin' });
  for (const argument of input.program) assert.ok(plist.includes(`<string>${argument}</string>`));
  assert.equal(service.serviceHomeOf(plist), input.home);
});
