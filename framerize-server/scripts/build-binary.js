import { execFileSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from 'node:fs';
import { isBuiltin } from 'node:module';
import path from 'node:path';
import { build } from 'esbuild';
import postject from 'postject';
import { artifactNames, currentTarget, digest, sourceIdentity } from './release-artifacts.js';

const [major, minor] = process.versions.node.split('.').map(Number);
if (major < 22 || (major === 22 && minor < 12)) throw new Error('Building requires Node 22.12 or newer');
const identity = sourceIdentity();
const target = currentTarget();
rmSync('dist/build.json', { force: true });
mkdirSync('dist', { recursive: true });
const bundle = await build({
  entryPoints: ['bin/framerize-server.js'], outfile: 'dist/bundle.cjs',
  bundle: true, platform: 'node', format: 'cjs', target: 'node22', metafile: true,
});
for (const output of Object.values(bundle.metafile.outputs)) {
  if (output.imports.some((entry) => entry.external && !isBuiltin(entry.path))) {
    throw new Error('Executable must not depend on external packages');
  }
}
writeFileSync('dist/sea-config.json', JSON.stringify({
  main: path.resolve('dist/bundle.cjs'), output: path.resolve('dist/sea-prep.blob'),
  disableExperimentalSEAWarning: true, useSnapshot: false, useCodeCache: false,
}));
execFileSync(process.execPath, ['--experimental-sea-config', 'dist/sea-config.json'], { stdio: 'inherit' });
const staged = 'dist/framerize-server.tmp';
try {
  copyFileSync(process.execPath, staged);
  chmodSync(staged, 0o755);
  if (process.platform === 'darwin') execFileSync('codesign', ['--remove-signature', staged], { stdio: 'inherit' });
  await postject.inject(staged, 'NODE_SEA_BLOB', readFileSync('dist/sea-prep.blob'), {
    sentinelFuse: 'NODE_SEA_FUSE_fce680ab2cc467b6e072b8b5df1996b2',
    machoSegmentName: process.platform === 'darwin' ? 'NODE_SEA' : undefined,
  });
  if (process.platform === 'darwin') execFileSync('codesign', ['--sign', '-', staged], { stdio: 'inherit' });
  const executable = `dist/${artifactNames(target)[0]}`;
  renameSync(staged, executable);
  writeFileSync('dist/build.json', `${JSON.stringify({ ...identity, target,
    node: process.version, executableSha256: digest(executable),
  }, null, 2)}\n`);
  process.stdout.write(`Built ${executable}\n`);
} finally { rmSync(staged, { force: true }); }
