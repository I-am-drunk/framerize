import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { copyFileSync, mkdirSync, writeFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const result = await build({ absWorkingDir: root, bundle: true, write: false,
  platform: 'node', format: 'esm', target: 'node22', minify: true, legalComments: 'none', metafile: true,
  stdin: { contents: "export { createInstance } from './framerize-core/src/instance.ts';\n" +
    "export { classifyReadiness } from './framerize-core/src/readiness.ts';",
    resolveDir: root, sourcefile: 'server-core-entry.ts', loader: 'ts' },
});
const inputs = Object.keys(result.metafile!.inputs);
assert.ok(inputs.every((name) => name === 'server-core-entry.ts' || name.startsWith('framerize-core/src/')));
assert.ok(Object.values(result.metafile!.outputs).every((output) =>
  output.imports.every((dependency) => dependency.external && dependency.path.startsWith('node:'))));
assert.equal(result.outputFiles.length, 1);
const bytes = result.outputFiles[0]!.contents;
assert.ok(bytes.length <= 64 * 1024, 'Server core bundle exceeds 64 KiB');
mkdirSync(new URL('../../framerize-server/dist/', import.meta.url), { recursive: true });
writeFileSync(new URL('../../framerize-server/dist/core.js', import.meta.url), bytes);
const declarations = new URL('../../framerize-server/dist/core-types/', import.meta.url);
mkdirSync(declarations, { recursive: true });
for (const name of ['backend', 'readiness', 'instance', 'instance-options', 'http-policy']) {
  copyFileSync(new URL(`../../framerize-core/dist/${name}.d.ts`, import.meta.url),
    new URL(`${name}.d.ts`, declarations));
}
writeFileSync(new URL('../../framerize-server/dist/core.d.ts', import.meta.url),
  "export { createInstance } from './core-types/instance.js';\n" +
  "export { classifyReadiness } from './core-types/readiness.js';\n" +
  "export type { FramerizeInstance } from './core-types/instance.js';\n" +
  "export type { InstanceOptions, RequestEvent } from './core-types/instance-options.js';\n" +
  "export type { BackendReadiness, InferenceBackend, InferenceRequest, InferenceResponse, RequestContext } from './core-types/backend.js';\n");
console.log(JSON.stringify({ bundle: 'framerize-server/dist/core.js', bytes: bytes.length,
  sha256: createHash('sha256').update(bytes).digest('hex'), inputs }));
