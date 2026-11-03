import { build } from 'esbuild';

await build({
  entryPoints: ['bin/framer-next.js'],
  outfile: 'dist/bundle.cjs',
  bundle: true,
  platform: 'node',
  format: 'cjs',
  target: 'node22',
  banner: { js: '#!/usr/bin/env node' },
});
