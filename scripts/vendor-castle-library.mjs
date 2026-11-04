import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { appendFileSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs';
import { dirname, posix, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, transform, version } from 'esbuild';

const commit = '2a15785963413d90ed79d7b1f4a6bd8f21828eb4';
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = process.argv[2];
const check = process.argv[3] === '--check';
assert.ok(repository && (process.argv.length === 3 || check && process.argv.length === 4),
  'Usage: node scripts/vendor-castle-library.mjs /path/to/castle [--check]');
const sources = {};
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
function source(path) {
  const bytes = execFileSync('git', ['-C', repository, 'show', `${commit}:typescript-castle/${path}`],
    { maxBuffer: 8 * 1024 * 1024 });
  sources[path] = digest(bytes);
  return bytes.toString('utf8');
}
function replace(text, before, after) {
  assert.equal(text.split(before).length, 2, `Expected one source boundary: ${before}`);
  return text.replace(before, after);
}
function write(path, text) {
  if (check) { assert.equal(readFileSync(path, 'utf8'), text, `Generated file differs: ${path}`); return; }
  const temporary = `${path}.tmp`;
  writeFileSync(temporary, '');
  const lines = text.match(/[^\n]*\n|[^\n]+$/g) ?? [];
  for (let offset = 0; offset < lines.length; offset += 39) {
    appendFileSync(temporary, lines.slice(offset, offset + 39).join(''));
  }
  renameSync(temporary, path);
}

const modules = new Map();
modules.set('location', `import { fileURLToPath } from 'node:url';
export const rootURL = new URL(process.platform === 'win32'
  ? 'file:///C:/__castle_library__/' : 'file:///__castle_library__/');
export const root = fileURLToPath(rootURL);`);
const manifestText = source('src/asset-manifest.ts');
const manifest = JSON.parse(manifestText.slice(manifestText.indexOf('['), manifestText.lastIndexOf(']') + 1)
  .replace(/,\s*]/g, ']'));
const assets = {};
for (const entry of manifest) {
  const bytes = Buffer.from(source(entry.path));
  assert.equal(bytes.length, entry.size, entry.path);
  assert.equal(digest(bytes), entry.sha256, entry.path);
  assets[entry.path] = bytes.toString('base64');
}
modules.set('assets', `const assets = ${JSON.stringify(assets)};
export const embeddedAsset = path => Buffer.from(assets[path], 'base64');`);
const changes = new Map();
function adapt(path, text) {
  for (const [before, after] of changes.get(path) ?? []) text = replace(text, before, after);
  return text;
}
const dataURL = code => `data:text/javascript;base64,${Buffer.from(code).toString('base64')}`;
const loader = dataURL(await bundle("export * from 'castle:src/snapshot-loader.ts';"));
changes.set('src/collection-guard.ts', [[
  "new URL('./snapshot-loader.js', import.meta.url)", `new URL(${JSON.stringify(loader)})`,
]]);
changes.set('src/asset-snapshot.ts', [
  ["import fs from 'node:fs';", "import fs from 'node:fs';\nimport { embeddedAsset } from 'castle:assets';"],
  ['const canonicalRoot = fs.realpathSync(root);', 'const canonicalRoot = root;'],
  ['if (fs.realpathSync(path) !== path) throw new Error();', ''],
  ['const bytes = readBounded(path, entry.size);', 'const bytes = embeddedAsset(entry.path);'],
  ['const canonicalTrace = fs.realpathSync(tracePath);',
    'const canonicalTrace = files.has(resolve(tracePath)) ? resolve(tracePath) : fs.realpathSync(tracePath);'],
]);
changes.set('src/collect-worker.ts', [
  ["import vm from 'node:vm';", "import vm from 'node:vm';\nimport { root, rootURL } from 'castle:location';"],
  ["fileURLToPath(new URL('../', import.meta.url))", 'root'],
  ["new URL('../castle/', import.meta.url)", "new URL('castle/', rootURL)"],
  ['\ntry {', '\nvoid (async () => {\ntry {'],
  ['parentPort?.postMessage({ failed: true });\n}',
    'parentPort?.postMessage({ failed: true });\n}\n})();'],
]);
const collector = dataURL(await bundle("import 'castle:src/collect-worker.ts';"));
const encoder = dataURL(await bundle("import 'castle:src/encode-worker.ts';"));
changes.set('src/collect.ts', [[
  "new URL('./collect-worker.js', import.meta.url)", `new URL(${JSON.stringify(collector)})`,
]]);
changes.set('src/encode.ts', [[
  "new URL('./encode-worker.js', import.meta.url)", `new URL(${JSON.stringify(encoder)})`,
]]);
changes.set('src/options.ts', [
  ["import { CastleError } from './errors.js';",
    "import { CastleError } from './errors.js';\nimport { rootURL } from 'castle:location';"],
  ["new URL('../castle/inputs/interaction-trace.json', import.meta.url)",
    "new URL('castle/inputs/interaction-trace.json', rootURL)"],
]);
async function bundle(contents) {
  const result = await build({ bundle: true, write: false, platform: 'node',
    format: 'esm', target: 'node22', legalComments: 'none',
    stdin: { contents, loader: 'ts', resolveDir: root },
    plugins: [{ name: 'pinned-castle', setup(builder) {
      builder.onResolve({ filter: /^node:/ }, args => ({ path: args.path, external: true }));
      builder.onResolve({ filter: /^castle:/ }, args => ({ path: args.path.slice(7), namespace: 'castle' }));
      builder.onResolve({ filter: /^\.\.?\//, namespace: 'castle' }, args => ({
        path: posix.join(posix.dirname(args.importer), args.path).replace(/\.js$/, '.ts'),
        namespace: 'castle',
      }));
      builder.onLoad({ filter: /.*/, namespace: 'castle' }, args => ({
        contents: modules.get(args.path) ?? adapt(args.path, source(args.path)), loader: 'ts',
      }));
    } }],
  });
  return (await transform(result.outputFiles[0].text, { loader: 'js', target: 'node22',
    legalComments: 'none' })).code;
}

let code = await bundle(`export * from 'castle:src/castle.ts';
export { createMinter } from 'castle:src/lifecycle.ts';
export { resolveOptions } from 'castle:src/options.ts';
export { encodeFrame } from 'castle:src/encoder/encode-frame.ts';
export { encode } from 'castle:src/encode.ts';
export { runCollectionWorker, runEncodingWorker } from 'castle:src/worker.ts';
export { privateEnvironment } from 'castle:src/environment.ts';
export { installCollectionGuard } from 'castle:src/collection-guard.ts';
export { decodeCollection } from 'castle:src/encoder/collection-values.ts';
export { encodeValues } from 'castle:src/encoder/encode-values.ts';
export { serializeValues } from 'castle:src/encoder/serialize.ts';
export { compress, MAX_COMPRESSION_BYTES } from 'castle:src/encoder/compress.ts';
export { huffmanCodes, huffmanLengths } from 'castle:src/encoder/compression-huffman.ts';
export { tokenize } from 'castle:src/encoder/compression-matches.ts';
export { envelope, encodeText, MAX_ENVELOPE_BYTES } from 'castle:src/encoder/envelope.ts';`);
code = replace(code, 'function createCastleMinter(input = {})',
  'function createCastleMinter(input: CastleOptions = {}): CastleMinter');
code = replace(code, 'function createMinter(options, operations)',
  'function createMinter(options: ResolvedCastleOptions, operations: CastleOperations): CastleMinter');
code = replace(code, 'function resolveOptions(input = {})',
  'function resolveOptions(input: CastleOptions = {}): ResolvedCastleOptions');
code = replace(code, 'var CastleError = class extends Error {',
  'class CastleError extends Error {\n  declare readonly code: keyof typeof messages;');
code = replace(code, 'constructor(code)', 'constructor(code: keyof typeof messages)');
for (const [before, after] of [
  ['encode(path, args, frame, signal, directory)',
    'encode(path: string | undefined, args: readonly string[], frame: Buffer, signal: AbortSignal, directory: string): Promise<string>'],
  ['encodeFrame(input, timestampMs)', 'encodeFrame(input: Uint8Array, timestampMs?: number): string'],
  ['runCollectionWorker(script, data, directory, signal, budgetMs)',
    'runCollectionWorker(script: URL, data: unknown, directory: string, signal: AbortSignal, budgetMs: number): Promise<Collection>'],
  ['runEncodingWorker(script, frame, directory, signal)',
    'runEncodingWorker(script: URL, frame: Uint8Array, directory: string, signal: AbortSignal): Promise<string>'],
  ['privateEnvironment(directory)', 'privateEnvironment(directory: string): NodeJS.ProcessEnv'],
  ['installCollectionGuard(snapshot)', 'installCollectionGuard(snapshot: ReadonlyMap<string, Buffer>): void'],
  ['decodeCollection(text2)', 'decodeCollection(text2: string): CollectedValue[]'],
  ['encodeValues(values, timestampMs)', 'encodeValues(values: readonly CollectedValue[], timestampMs?: number): string'],
  ['serializeValues(values)', 'serializeValues(values: readonly CollectedValue[]): Buffer'],
  ['compress(input)', 'compress(input: Uint8Array): Buffer'],
  ['huffmanCodes(lengths, maximum)', 'huffmanCodes(lengths: readonly number[], maximum: number): number[]'],
  ['huffmanLengths(frequencies, maximum)', 'huffmanLengths(frequencies: readonly number[], maximum: number): number[]'],
  ['tokenize(input)', 'tokenize(input: Uint8Array): CompressionToken[]'],
  ['envelope(payload, timestampMs)', 'envelope(payload: Uint8Array, timestampMs: number): string'],
  ['encodeText(text2)', 'encodeText(text2: string): string'],
]) code = replace(code, `function ${before}`, `function ${after}`);
const types = source('src/types.ts') + `
${source('src/encoder/serialize.ts').match(/^export type CollectedValue = .+;/m)[0]}
${source('src/encoder/compression-matches.ts').match(/^export type CompressionToken = .+;/m)[0]}
export interface ResolvedCastleOptions {
  readonly encoderPath: string | undefined;
  readonly encoderArgs: readonly string[];
  readonly tracePath: string;
  readonly budgetMs: number;
  readonly timeoutMs: number;
}
export interface CastleOperations {
  collect(options: { directory: string; tracePath: string; cuid?: string },
    signal: AbortSignal, budgetMs: number): Promise<Collection>;
  encode(path: string | undefined, args: readonly string[], frame: Buffer,
    signal: AbortSignal, directory: string): Promise<string>;
}
`;
const generated = '// @ts-nocheck\n' + types + code;
write(resolve(root, 'framerize-server/src/castle.ts'), generated);
mkdirSync(resolve(root, 'framerize-server/fixtures/castle'), { recursive: true });
for (const name of ['lifecycle', 'operations', 'integration', 'encoding-worker', 'collection',
  'compress', 'envelope', 'serialize-values', 'fixtures/worker', 'fixtures/encoder', 'fixtures/guard',
  'fixtures/encoding-worker', 'fixtures/collection-inputs', 'fixtures/compression-inputs',
  'fixtures/envelope-inputs', 'fixtures/encoding-inputs']) {
  let input = source(`checks/${name}.ts`);
  if (name === 'integration') input = replace(input,
    "const entry = process.env.CASTLE_ENTRY ?? fileURLToPath(new URL('../../dist/castle.js', import.meta.url));\n"
      + "const { createCastleMinter } = await import(pathToFileURL(entry).href) as typeof Library;",
    "import { createCastleMinter } from '../dist/castle.js';");
  const fixture = name.startsWith('fixtures/');
  input = input.replace(/from '\.\.\/(?:\.\.\/)?src\/[^']+\.js'/g,
    `from '${fixture ? '../../' : '../'}dist/castle.js'`);
  input = input.replaceAll("new URL('./fixtures/", "new URL('../fixtures/castle/");
  input = input.replaceAll("from './fixtures/", "from '../fixtures/castle/");
  const output = (await transform(input, { loader: 'ts', target: 'node22', legalComments: 'none' })).code;
  const path = fixture ? `fixtures/castle/${name.slice(9)}.js` : `test/castle-${name}.test.js`;
  write(resolve(root, `framerize-server/${path}`), output);
}
const orderedSources = Object.fromEntries(Object.entries(sources).sort(([left], [right]) => left.localeCompare(right)));
const receipt = { commit, esbuild: version, sources: orderedSources, sha256: digest(generated),
  generatorSha256: digest(readFileSync(fileURLToPath(import.meta.url))),
  workers: { collector: digest(collector), encoder: digest(encoder), loader: digest(loader) } };
write(resolve(root, 'framerize-server/test/helpers/castle-library-source.json'),
  JSON.stringify(receipt, null, 2) + '\n');
console.log(JSON.stringify({ sourceFiles: Object.keys(sources).length, sha256: receipt.sha256 }));
