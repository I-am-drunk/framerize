import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { serviceStatus } from './service-control.ts'; export { selectServiceManager } from './service-units.ts';",
  resolveDir: fileURLToPath(new URL('../src/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false });
const bytes = compiled.outputFiles[0].contents;
const bundleSha256 = createHash('sha256').update(bytes).digest('hex');
const { serviceStatus, selectServiceManager } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

for (const variant of ['unset', 'absolute', 'empty', 'relative']) {
  test(`Server service status honors ${variant} XDG_CONFIG_HOME`, { timeout: 3000 }, async context => {
    const directory = await mkdtemp(join(tmpdir(), 'server-xdg-'));
    const previousCwd = process.cwd();
    const cleanup = async () => { process.chdir(previousCwd); await rm(directory, { recursive: true, force: true }); };
    context.after(cleanup);
    const homeDir = join(directory, 'home'), cwd = join(directory, 'cwd');
    await mkdir(cwd);
    process.chdir(cwd);
    const xdgConfigHome = variant === 'unset' ? undefined : variant === 'absolute'
      ? join(directory, 'custom-config') : variant === 'empty' ? '' : 'relative-config';
    const plan = { home: join(directory, 'server'), homeDir, xdgConfigHome,
      program: [process.execPath, '/synthetic/server.js', 'serve'],
      environmentPath: '/synthetic/bin', platform: 'linux', uid: 1234 };
    const manager = selectServiceManager(plan);
    const expected = join(variant === 'absolute' ? xdgConfigHome : join(homeDir, '.config'),
      'systemd', 'user', 'framerize-server.service');
    await mkdir(dirname(expected), { recursive: true, mode: 0o700 });
    await writeFile(expected, manager.contents, { mode: 0o600 });
    const before = (await readdir(directory, { recursive: true })).sort();
    const calls = [];
    const state = await serviceStatus(plan, async (command, args) => {
      calls.push([command, [...args]]);
      assert.equal(command, 'systemctl');
      assert.equal(args[0], '--user');
      assert.equal(args[2], 'framerize-server.service');
      if (args[1] === 'is-enabled') {
        assert.equal(args.length, 3);
        return { code: 0, stdout: 'enabled' };
      }
      assert.deepEqual(args.slice(1), ['show', 'framerize-server.service', '--property=ActiveState,SubState,MainPID']);
      return { code: 0, stdout: 'ActiveState=inactive\nSubState=dead\nMainPID=0\n' };
    });
    assert.equal(await readFile(expected, 'utf8'), manager.contents);
    assert.deepEqual((await readdir(directory, { recursive: true })).sort(), before);
    await cleanup();
    await assert.rejects(readdir(directory), { code: 'ENOENT' });
    context.diagnostic(JSON.stringify({ variant, bundleSha256, expected, selected: manager.path,
      state, calls, unchangedFiles: true, privateStateRemoved: true }));
    assert.equal(manager.path, expected);
    assert.equal(state.unitPath, expected);
    assert.equal(state.installed, true);
    assert.equal(state.current, true);
    assert.equal(state.otherHome, false);
    assert.equal(state.running, false);
    assert.equal(calls.length, 2);
  });
}
