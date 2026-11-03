import { rm } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { build } from 'esbuild';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
import { dirname, join } from 'node:path';
import { mkdir, readdir, readFile, writeFile } from 'node:fs/promises';
import assert from 'node:assert/strict';

const root = fileURLToPath(new URL('../', import.meta.url));
const require = createRequire(import.meta.url);

await rm(`${root}dist`, { recursive: true, force: true });

execFileSync(process.execPath, [require.resolve('typescript/bin/tsc'), '-p', join(root, 'tsconfig.json')], {
  stdio: 'inherit',
});

const result = await build({
  absWorkingDir: root,
  entryPoints: [`${root}src/index.ts`],
  minify: true,
  metafile: true,
  outdir: `${root}dist`,
  outExtension: { '.js': '.js' },
  bundle: true,
  platform: 'node',
  format: 'esm',
  target: 'node22.12',
  packages: 'bundle',
  legalComments: 'inline',
});
assert.ok(Object.values(result.metafile.outputs).every((output) =>
  output.imports.every((dependency) => dependency.external && dependency.path.startsWith('node:'))));
await writeFile(join(root, 'dist/cli.js'), "export { run } from './index.js';\n");

console.log('build complete: dist/cli.js + dist/index.js + declarations');
const output = join(root, 'dist');
const core = dirname(fileURLToPath(import.meta.resolve('@framerize/core')));
await mkdir(join(output, 'core-types'), { recursive: true });
for (const name of await readdir(core)) {
  if (name.endsWith('.d.ts')) {
    await writeFile(join(output, 'core-types', name), await readFile(join(core, name)));
  }
}
for (const name of await readdir(output)) {
  if (!name.endsWith('.d.ts')) continue;
  const declaration = await readFile(join(output, name), 'utf8');
  await writeFile(join(output, name), declaration.replaceAll("'@framerize/core'", "'./core-types/index.js'"));
}
