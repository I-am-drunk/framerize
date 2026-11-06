import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
import { access, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/platform.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { resolveCommandPath } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);
const posix = { skip: process.platform === 'win32' && 'POSIX environment case sensitivity', timeout: 5000 };

for (const name of ['Path', 'path', 'PATH']) {
  test(`POSIX command discovery respects ${name} case sensitivity`, posix, async context => {
    const root = await mkdtemp(join(tmpdir(), 'framerize-posix-path-env-'));
    context.after(() => rm(root, { recursive: true, force: true }));
    const command = `framerize-synthetic-${randomUUID()}`;
    const executable = join(root, command);
    await writeFile(executable, '#!/bin/sh\nprintf "owned-fixture\\n"\n', { mode: 0o700 });
    const env = { [name]: root };
    const original = { ...env };
    const child = spawnSync(command, [], { env, cwd: root, encoding: 'utf8',
      timeout: 1000, killSignal: 'SIGKILL' });
    const selected = resolveCommandPath(command, process.platform, env);
    await rm(root, { recursive: true, force: true });
    await assert.rejects(access(root), { code: 'ENOENT' });
    assert.deepEqual(env, original);
    context.diagnostic(JSON.stringify({ name, childStatus: child.status,
      childError: child.error?.code, resolverSelected: selected !== null, privateStateRemoved: true }));
    if (name === 'PATH') {
      assert.ifError(child.error);
      assert.equal(child.status, 0);
      assert.equal(child.signal, null);
      assert.equal(child.stdout, 'owned-fixture\n');
      assert.equal(selected, executable);
    } else {
      assert.equal(child.error?.code, 'ENOENT');
      assert.equal(child.status, null);
      assert.equal(selected, null, `${name} is not the POSIX PATH variable`);
    }
  });
}
