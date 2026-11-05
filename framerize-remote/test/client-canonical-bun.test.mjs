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
const { planUpdate } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const packages = { claude: '@anthropic-ai/claude-code', codex: '@openai/codex' };

for (const [client, packageName] of Object.entries(packages)) {
  for (const layout of ['native-alias', 'generic-alias', 'foreign-package', 'package-suffix']) {
    test(`${client} canonical Bun lookup: ${layout}`, {
      timeout: 5000, skip: process.platform === 'win32' ? 'POSIX symlink layout' : false,
    }, async context => {
      const root = await mkdtemp(join(tmpdir(), 'frm-canonical-bun-'));
      context.after(() => rm(root, { recursive: true, force: true }));
      const home = await realpath(root);
      const canonicalPackage = layout === 'foreign-package' ? '@synthetic/unrelated'
        : layout === 'package-suffix' ? `${packageName}-other` : packageName;
      const target = join(home, '.bun', 'install', 'global', 'node_modules', canonicalPackage, 'cli.js');
      const resolvedPath = layout !== 'native-alias' ? join(home, 'bin', client)
        : client === 'claude' ? join(home, '.local', 'bin', client)
          : join(home, '.codex', 'packages', 'standalone', 'current', 'bin', client);
      await mkdir(dirname(target), { recursive: true, mode: 0o700 });
      await mkdir(dirname(resolvedPath), { recursive: true, mode: 0o700 });
      await writeFile(target, 'synthetic fixture, never executed\n', { mode: 0o600 });
      await symlink(target, resolvedPath);
      const realPath = await realpath(resolvedPath);
      assert.equal(realPath, target);
      const result = planUpdate(client, { resolvedPath, realPath, platform: 'linux', env: { HOME: home } });
      if (layout === 'foreign-package' || layout === 'package-suffix') {
        assert.deepEqual(result.owner, { kind: 'unknown' });
        assert.equal(result.kind, 'manual');
      } else {
        assert.deepEqual(result.owner, { kind: 'bun' });
        assert.equal(result.kind, 'command');
        assert.equal(result.command.command, 'bun');
        assert.deepEqual(result.command.args, ['i', '-g', `${packageName}@latest`]);
        assert.equal(result.command.env, undefined);
      }
    });
  }
}
