import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { isBuiltin } from 'node:module';
import { posix } from 'node:path';
import { build } from 'esbuild';
import ts from 'typescript';
import { readPackageArchive } from './package-archive.js';

export function verifyServerSource(bytes: Buffer, files: string[], readSource: (file: string) => Buffer): void {
  const archive = readPackageArchive(bytes);
  assert.deepEqual([...archive.keys()].sort(), files.map(file => `package/${file}`).sort(),
    'Server archive inventory differs from checkout');
  for (const file of files) {
    assert.deepEqual(archive.get(`package/${file}`), readSource(file), `Server archive differs: ${file}`);
  }
}

export async function bundleServerArchive(bytes: Buffer, expected: { name: string; version: string }) {
  const archive = readPackageArchive(bytes);
  const manifest = JSON.parse(archive.get('package/package.json')!.toString('utf8'));
  assert.equal(manifest.name, 'framerize-server');
  assert.equal(manifest.name, expected.name);
  assert.equal(manifest.version, expected.version);
  assert.equal(manifest.private, true);
  assert.deepEqual(manifest.bin, {
    framerize: './bin/framerize-server.js', 'framerize-server': './bin/framerize-server.js',
  });
  for (const field of ['dependencies', 'optionalDependencies', 'peerDependencies']) {
    assert.equal(Object.keys(manifest[field] ?? {}).length, 0, 'Unexpected runtime dependencies');
  }
  const inputs = new Set<string>();
  const result = await build({ entryPoints: ['package/bin/framerize-server.js'],
    bundle: true, write: false, platform: 'node', format: 'cjs', target: 'node22',
    metafile: true, logLevel: 'silent', define: { 'import.meta.url': 'undefined' },
    plugins: [{ name: 'server-archive', setup(builder) {
      builder.onResolve({ filter: /.*/ }, args => {
        if (isBuiltin(args.path)) return { path: args.path, external: true };
        assert.ok(args.kind === 'entry-point' || /^\.\.?\//.test(args.path), 'Non-relative package import');
        const path = args.kind === 'entry-point' ? args.path
          : posix.normalize(posix.join(posix.dirname(args.importer), args.path));
        assert.ok(archive.has(path), 'Import is absent from the verified archive');
        return { path, namespace: 'server-archive' };
      });
      builder.onLoad({ filter: /.*/, namespace: 'server-archive' }, args => {
        assert.match(args.path, /\.(?:js|mjs|json)$/);
        inputs.add(args.path);
        if (!args.path.endsWith('.json')) verifyImports(args.path, archive.get(args.path)!.toString('utf8'));
        return { contents: archive.get(args.path)!, loader: args.path.endsWith('.json') ? 'json' : 'js' };
      });
    } }] });
  assert.equal(result.warnings.length, 0, 'Native bundle has unresolved build warnings');
  assert.equal(result.outputFiles.length, 1);
  for (const output of Object.values(result.metafile!.outputs)) {
    assert.ok(output.imports.every(entry => entry.external && isBuiltin(entry.path)),
      'Native bundle depends on an external package');
  }
  return { code: result.outputFiles[0]!.contents, inputs: [...inputs].sort(),
    packageSha256: createHash('sha256').update(bytes).digest('hex'), packageBytes: bytes.length };
}

function verifyImports(path: string, text: string): void {
  const source = ts.createSourceFile(path, text, ts.ScriptTarget.ES2022, true, ts.ScriptKind.JS);
  const visit = (node: ts.Node): void => {
    if (ts.isCallExpression(node) && (node.expression.kind === ts.SyntaxKind.ImportKeyword ||
        ts.isIdentifier(node.expression) && node.expression.text === 'require')) {
      const argument = node.arguments[0];
      assert.ok(node.arguments.length === 1 && ts.isStringLiteral(argument!),
        'Computed module import');
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
}
