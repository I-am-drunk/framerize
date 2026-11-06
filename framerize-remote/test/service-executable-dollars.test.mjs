import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/serviceUnits.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const service = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const executables = [
  ['/opt/${NODE_HOME}/node', '/opt/${NODE_HOME}/node'],
  ['/opt/cash$$/node', '/opt/cash$$/node'],
  ['/opt/100% cash ${NODE_HOME}/node', '"/opt/100%% cash ${NODE_HOME}/node"'],
];

for (const [executable, encoded] of executables) {
  test(`systemd retains executable path ${executable} while escaping later arguments`, () => {
    const unit = service.renderServiceUnit({
      program: [executable, '/opt/${SCRIPT_HOME}/frm.js', 'serve'],
      home: '/synthetic/profile', logPath: '/synthetic/profile/service.log',
    });
    const command = unit.split('\n').find(line => line.startsWith('ExecStart='));
    assert.equal(command, `ExecStart=${encoded} /opt/$\${SCRIPT_HOME}/frm.js serve`);
  });
}
