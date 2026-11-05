import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, realpath, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/maintenance.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { planUpdate } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const packages = { claude: '@anthropic-ai/claude-code', codex: '@openai/codex' };

for (const [client, packageName] of Object.entries(packages)) {
  for (const layout of ['npm-alias', 'npm-prefix', 'mise-alias', 'bun']) {
    test(`${client} updater preserves ${layout} ownership through a Bun-looking launcher`, {
      timeout: 5000, skip: process.platform === 'win32' ? 'POSIX symlink-layout fixture' : false,
    }, async context => {
      const root = await mkdtemp(join(tmpdir(), 'frm-owner-'));
      context.after(() => rm(root, { recursive: true, force: true }));
      const home = await realpath(root);
      const prefix = join(home, layout === 'npm-prefix' ? '.bun' : 'npm-global');
      const resolvedPath = join(home, '.bun', 'bin', client);
      const target = layout.startsWith('npm')
        ? join(prefix, 'lib', 'node_modules', packageName, 'cli.js')
        : layout === 'mise-alias'
          ? join(home, '.local', 'share', 'mise', 'installs', client, '1.0.0', 'bin', client)
          : join(home, '.bun', 'install', 'global', 'node_modules', packageName, 'cli.js');
      await mkdir(dirname(target), { recursive: true, mode: 0o700 });
      await mkdir(dirname(resolvedPath), { recursive: true, mode: 0o700 });
      await writeFile(target, 'synthetic fixture; never executed\n', { mode: 0o600 });
      await symlink(target, resolvedPath);
      const realPath = await realpath(resolvedPath);
      assert.equal(realPath, target);
      const result = planUpdate(client, { resolvedPath, realPath, platform: 'linux', env: { HOME: home } });
      context.diagnostic(JSON.stringify({ layout, owner: result.owner, kind: result.kind }));
      if (layout.startsWith('npm')) {
        assert.deepEqual(result.owner, { kind: 'npm', prefix });
        assert.equal(result.kind, 'command');
        assert.equal(result.command.command, 'npm');
        assert.deepEqual(result.command.args, ['install', '-g', '--prefix', prefix,
          `--allow-scripts=${packageName}`, `${packageName}@latest`]);
      } else if (layout === 'mise-alias') {
        assert.deepEqual(result, { kind: 'manual', owner: { kind: 'mise' },
          reason: 'Update the version pinned in your mise configuration.' });
      } else {
        assert.deepEqual(result.owner, { kind: 'bun' });
        assert.equal(result.kind, 'command');
        assert.equal(result.command.command, 'bun');
        assert.deepEqual(result.command.args, ['i', '-g', `${packageName}@latest`]);
      }
    });
  }
}
