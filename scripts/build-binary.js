import { execFileSync } from 'node:child_process';
import { chmodSync, copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { build } from 'esbuild';
import postject from 'postject';

const [major, minor] = process.versions.node.split('.').map(Number);
if (major < 22 || (major === 22 && minor < 12)) {
  throw new Error('building the executable requires Node 22.12 or newer');
}
mkdirSync('dist', { recursive: true });
await build({
  entryPoints: ['bin/framer-next.js'],
  outfile: 'dist/bundle.cjs',
  bundle: true,
  platform: 'node',
  format: 'cjs',
  target: 'node22',
});
writeFileSync('dist/sea-config.json', JSON.stringify({
  main: path.resolve('dist/bundle.cjs'),
  output: path.resolve('dist/sea-prep.blob'),
  disableExperimentalSEAWarning: true,
  useSnapshot: false,
  useCodeCache: false,
}));
copyFileSync(new URL('../LICENSE', import.meta.url), 'dist/LICENSE');
execFileSync(process.execPath, ['--experimental-sea-config', 'dist/sea-config.json'], { stdio: 'inherit' });
chmodSync('dist/framer-next', 0o755);
copyFileSync(process.execPath, 'dist/framer-next');
chmodSync('dist/framer-next', 0o755);
await postject.inject('dist/framer-next', 'NODE_SEA_BLOB', readFileSync('dist/sea-prep.blob'), {
  sentinelFuse: 'NODE_SEA_FUSE_fce680ab2cc467b6e072b8b5df1996b2',
});
console.log('Built dist/framer-next');
