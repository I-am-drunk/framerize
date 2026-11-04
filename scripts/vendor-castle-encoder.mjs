import { build, transform, version } from 'esbuild';
import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, appendFileSync, renameSync } from 'node:fs';
import { resolve, relative, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const source = process.argv[2];
if (!source || process.argv.length !== 3) {
  throw new Error('Usage: node scripts/vendor-castle-encoder.mjs /path/to/typescript-castle');
}
const reference = resolve(source);
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const result = await build({
  absWorkingDir: reference,
  entryPoints: ['src/encoder/encode-frame.ts'],
  bundle: true, write: false, metafile: true,
  format: 'iife', globalName: 'CastleEncoder',
  platform: 'node', target: 'node22', legalComments: 'none',
  plugins: [{
    name: 'shared-node-types',
    setup(builder) {
      builder.onResolve({ filter: /^node:util\/types$/ }, () => ({
        path: 'node-types', namespace: 'shared-node-types',
      }));
      builder.onLoad({ filter: /.*/, namespace: 'shared-node-types' }, () => ({
        contents: 'export const isUint8Array = types.isUint8Array;', loader: 'js',
      }));
    },
  }],
});
const encoder = (await transform(result.outputFiles[0].text, {
  loader: 'js', target: 'node22', legalComments: 'none',
})).code.trimEnd();
const target = resolve(root, 'framerize-server/src/castle.ts');
const current = readFileSync(target, 'utf8');
const start = current.indexOf('var CastleEncoder = ');
const end = current.indexOf('function createCastleMinter(input = {}) {');
if (end < 0 || start > end) throw new Error('Castle source boundary not found');
const generated = current.slice(0, start < 0 ? end : start)
  + encoder + '\n' + current.slice(end);
const temporary = `${target}.tmp`;
writeFileSync(temporary, '');
const lines = generated.split('\n');
for (let offset = 0; offset < lines.length; offset += 40) {
  const chunk = lines.slice(offset, offset + 40).join('\n');
  appendFileSync(temporary, chunk + (offset + 40 < lines.length ? '\n' : ''));
}
renameSync(temporary, target);
const files = Object.keys(result.metafile.inputs)
  .filter(path => !path.startsWith('shared-node-types:')).sort();
const sources = Object.fromEntries(files.map(path => [
  relative(reference, resolve(reference, path)), hash(readFileSync(resolve(reference, path))),
]));
const receipt = { esbuild: version, sources, encoderSha256: hash(encoder + '\n') };
writeFileSync(resolve(root, 'framerize-server/test/helpers/castle-encoder-source.json'),
  JSON.stringify(receipt, null, 2) + '\n');
console.log(JSON.stringify({ sourceFiles: files.length, encoderSha256: receipt.encoderSha256 }));
