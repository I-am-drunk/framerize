import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, mkdir, writeFile, readFile, symlink, rm, access } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const execute = promisify(execFile);
const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));

for (const [client, relative] of [
  ['claude', '.local/bin/claude'], ['claude', '.bun/bin/claude'],
  ['codex', '.codex/packages/standalone/current/bin/codex'], ['codex', '.bun/bin/codex'],
]) {
  test(`installed update leaves the Homebrew owner of ${relative} in control`, {
    skip: process.platform === 'win32' && 'Homebrew ownership uses POSIX symlinks', timeout: 10000,
  }, async t => {
    const root = await mkdtemp(join(tmpdir(), 'frm-homebrew-owner-'));
    t.after(() => rm(root, { recursive: true, force: true }));
    const home = join(root, 'home'), executable = join(home, relative);
    const cask = client === 'claude' ? 'claude-code' : 'codex';
    const target = join(root, 'brew/Caskroom', cask, '2.1.0', client);
    const state = join(home, 'profile'), profile = join(state, 'framerize-remote.json');
    const guard = join(root, 'guard.mjs'), attempts = join(root, 'attempts');
    await Promise.all([dirname(executable), dirname(target), state].map(path => mkdir(path, { recursive: true })));
    await writeFile(target, '#!/bin/sh\nexit 99\n', { mode: 0o700 });
    await symlink(target, executable);
    await writeFile(guard, `import processes from 'node:child_process';
import { appendFileSync } from 'node:fs';
import { syncBuiltinESMExports } from 'node:module';
const denied = () => {
  appendFileSync(${JSON.stringify(attempts)}, 'unexpected side effect\\n');
  throw new Error('Unexpected command or fetch');
};
processes.spawn = denied;
processes.execFile = denied;
globalThis.fetch = denied;
syncBuiltinESMExports();
`);
    const original = JSON.stringify({ remote: 'http://127.0.0.1:9', port: 8788,
      localToken: { kind: 'value', value: 'synthetic-local-token' } });
    await writeFile(profile, original, { mode: 0o600 });
    const { stdout, stderr } = await execute(process.execPath, ['--import', guard,
      join(packageRoot, 'bin/frm.js'), 'client', client, '--update'], {
      env: { HOME: home, USERPROFILE: home, FRM_HOME: state, PATH: dirname(executable) },
      timeout: 5000, maxBuffer: 65536,
    });
    assert.ok(stdout.includes(`updated outside Framerize: brew upgrade --cask ${cask}\n`), stdout);
    assert.equal(stderr, '');
    await assert.rejects(access(attempts), { code: 'ENOENT' });
    assert.equal(await readFile(profile, 'utf8'), original);
  });
}
