import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { installService } from './serviceControl.ts'; export { renderServiceUnit } from './serviceUnits.ts'; export { selectServiceManager } from './serviceManager.ts';",
  resolveDir: fileURLToPath(new URL('../src/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false });
const bytes = compiled.outputFiles[0].contents;
const bundleSha256 = createHash('sha256').update(bytes).digest('hex');
const { installService, renderServiceUnit, selectServiceManager } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

async function exercise(context, stopFails) {
  const directory = await mkdtemp(join(tmpdir(), 'frm-install-stop-'));
  let removed = false;
  const cleanup = async () => {
    if (removed) return;
    await rm(directory, { recursive: true, force: true });
    removed = true;
  };
  context.after(cleanup);
  const home = join(directory, 'profile');
  const host = { platform: 'linux', homeDir: directory, xdgConfigHome: join(directory, 'config'), uid: 1234 };
  const manager = selectServiceManager(host);
  const calls = [], preflight = [];
  let runningProgram = 'old', result, failure;
  const service = {
    home, host, program: [process.execPath, '/synthetic/new.mjs', 'serve'], environmentPath: '/synthetic/bin',
    async run(command, args) {
      calls.push([command, [...args]]);
      if (command === 'loginctl') {
        assert.deepEqual(args, ['show-user', '1234', '--property=Linger', '--value']);
        return { code: 0, stdout: 'yes\n' };
      }
      assert.equal(command, 'systemctl');
      assert.equal(args[0], '--user');
      if (args[1] === 'show-environment') return { code: 0, stdout: '' };
      if (args[1] === 'is-active') return { code: runningProgram ? 0 : 3, stdout: runningProgram ? 'active\n' : 'inactive\n' };
      if (args[1] === 'is-enabled') return { code: 0, stdout: 'enabled\n' };
      if (args[1] === 'stop') {
        if (stopFails) return { code: 1, stdout: '' };
        runningProgram = undefined;
      } else if (args[1] === 'restart') runningProgram = 'new';
      else assert.ok(['daemon-reload', 'enable'].includes(args[1]), `unexpected synthetic command ${args[1]}`);
      return { code: 0, stdout: '' };
    },
  };
  const previous = renderServiceUnit({ program: [process.execPath, '/synthetic/old.mjs', 'serve'],
    home, logPath: join(home, 'frm-service.log') });
  const next = renderServiceUnit({ program: service.program, home, logPath: join(home, 'frm-service.log') });
  const profile = '{"remote":"http://127.0.0.1:9","port":8788}\n';
  await mkdir(home, { recursive: true, mode: 0o700 });
  await mkdir(join(host.xdgConfigHome, 'systemd', 'user'), { recursive: true, mode: 0o700 });
  await writeFile(join(home, 'framerize-remote.json'), profile, { mode: 0o600 });
  await writeFile(manager.unitPath, previous, { mode: 0o600 });
  try {
    result = await installService(service, async () => { preflight.push(runningProgram ?? 'stopped'); });
  } catch (error) { failure = { code: error.code, message: error.message }; }
  const current = await readFile(manager.unitPath, 'utf8');
  assert.equal(await readFile(join(home, 'framerize-remote.json'), 'utf8'), profile);
  await cleanup();
  await assert.rejects(readFile(manager.unitPath), { code: 'ENOENT' });
  context.diagnostic(JSON.stringify({ bundleSha256, result, failure, calls, preflight,
    runningProgram, unitReplaced: current !== previous, privateStateRemoved: removed }));
  return { result, failure, calls, preflight, runningProgram, current, previous, next };
}

test('service installation stops after a failed stop without replacing the running unit', { timeout: 3000 }, async context => {
  const state = await exercise(context, true);
  assert.equal(state.failure?.code, 'service_failed');
  assert.equal(state.result, undefined);
  assert.deepEqual(state.preflight, []);
  assert.equal(state.current, state.previous);
  assert.equal(state.runningProgram, 'old');
  assert.equal(state.calls.filter(([, args]) => args[1] === 'stop').length, 1);
  assert.deepEqual(state.calls.filter(([, args]) => ['daemon-reload', 'enable', 'restart'].includes(args[1])), []);
});

test('service installation proceeds after a successful stop', { timeout: 3000 }, async context => {
  const state = await exercise(context, false);
  assert.equal(state.failure, undefined);
  assert.equal(state.result, 'updated');
  assert.deepEqual(state.preflight, ['stopped']);
  assert.equal(state.current, state.next);
  assert.equal(state.runningProgram, 'new');
  assert.deepEqual(state.calls.map(([, args]) => args[1])
    .filter(action => ['stop', 'daemon-reload', 'enable', 'restart'].includes(action)),
  ['stop', 'daemon-reload', 'enable', 'restart']);
});
