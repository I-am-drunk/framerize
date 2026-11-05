import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdtemp, mkdir, readFile, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

const execute = promisify(execFile);
const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const preload = fileURLToPath(new URL('./helpers/service-program-preload.mjs', import.meta.url));

for (const alias of ['frm', 'framerize']) {
  test(`${alias} service keeps its Remote launcher after the command alias changes`, {
    timeout: 15000, skip: process.platform !== 'linux' && 'Uses a synthetic systemd runner',
  }, async context => {
    const root = await realpath(await mkdtemp(join(tmpdir(), 'frm-service-program-')));
    context.after(() => rm(root, { recursive: true, force: true }));
    const home = join(root, 'profile'), config = join(root, 'config');
    const launcher = await realpath(join(packageRoot, 'bin/frm.js'));
    const shim = join(root, alias), other = join(root, 'other-role.mjs');
    await mkdir(home);
    const profile = JSON.stringify({ remote: 'http://127.0.0.1:1', port: 8788 });
    await writeFile(join(home, 'framerize-remote.json'), profile);
    await writeFile(other, "console.log('wrong-package-role');\n");
    await symlink(launcher, shim);
    const env = { HOME: root, FRM_HOME: home, XDG_CONFIG_HOME: config, PATH: '',
      NODE_DISABLE_COMPILE_CACHE: '1' };
    const run = (entry, args) => execute(process.execPath, ['--import', preload, entry, ...args],
      { cwd: root, env, timeout: 5000, maxBuffer: 64 * 1024 });
    const installed = await run(shim, ['service', 'install', '--json']);
    assert.deepEqual(JSON.parse(installed.stdout), { service: 'installed' });
    assert.equal(installed.stderr, '');
    const unitPath = join(config, 'systemd/user/frm.service');
    const unit = await readFile(unitPath, 'utf8');
    const words = /^ExecStart=(.*)$/m.exec(unit)?.[1].match(/"(?:[^"\\]|\\.)*"|\S+/g);
    assert.ok(words, 'The service must contain its executable command');
    const program = words.map(word => (word.startsWith('"') ? JSON.parse(word) : word).replaceAll('%%', '%'));
    await rm(shim);
    await symlink(other, shim);
    const result = await execute(program[0], [program[1], '--help'],
      { cwd: root, env, timeout: 5000, maxBuffer: 64 * 1024 });
    assert.match(result.stdout, /frm connect \[LINK\]/);
    assert.doesNotMatch(result.stdout, /wrong-package-role/);
    assert.equal(result.stderr, '');
    assert.deepEqual(program, [process.execPath, launcher, 'serve']);
    const removed = await run(launcher, ['service', 'uninstall', '--json']);
    assert.deepEqual(JSON.parse(removed.stdout), { service: 'removed' });
    assert.equal(removed.stderr, '');
    await assert.rejects(readFile(unitPath), { code: 'ENOENT' });
    assert.equal(await readFile(join(home, 'framerize-remote.json'), 'utf8'), profile);
  });
}
