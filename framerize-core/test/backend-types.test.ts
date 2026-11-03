import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

function checkTypes(source: string): readonly ts.Diagnostic[] {
  const fileName = fileURLToPath(new URL('../../contract-check.ts', import.meta.url));
  const options: ts.CompilerOptions = {
    strict: true, noEmit: true, exactOptionalPropertyTypes: true,
    target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.NodeNext,
    moduleResolution: ts.ModuleResolutionKind.NodeNext, types: ['node'],
  };
  const host = ts.createCompilerHost(options);
  const original = host.getSourceFile.bind(host);
  host.getSourceFile = (name, languageVersion, onError, shouldCreateNewSourceFile) =>
    name === fileName ? ts.createSourceFile(name, source, languageVersion, true)
      : original(name, languageVersion, onError, shouldCreateNewSourceFile);
  const program = ts.createProgram([fileName], options, host);
  return ts.getPreEmitDiagnostics(program);
}

test('both backend locations satisfy one contract and invalid uses fail type checking', () => {
  const source = [
    "import type { InferenceBackend, InferencePath, RequestContext, Readiness } from './src/index.js';",
    "import { createInstance } from './src/index.js';",
    "const body = async function* () { yield Buffer.from('synthetic'); };",
    'const direct: InferenceBackend = {',
    '  async forward(request) { return { status: 200, headers: {}, body: request.body }; },',
    '  async ready() { return true; }, close() {},',
    '};',
    'const remote: InferenceBackend = { ...direct, async forward(request) {',
    '  return { status: 200, headers: {}, body: request.body };',
    '} };',
    'for (const backend of [direct, remote]) {',
    '  const instance = createInstance({ backend, token: "synthetic-token" });',
    '  void backend.forward({ path: "/v1/responses", headers: {}, body: body(),',
    '    signal: new AbortController().signal });',
    '}',
    'const invalidPath: InferencePath = "/admin";',
    'const missingSignal: RequestContext = {};',
    'const invalidReady: Readiness = { state: "ready", reason: "upstream_unavailable" };',
  ].join('\n');
  const diagnostics = checkTypes(source);
  assert.equal(diagnostics.length, 3, ts.formatDiagnostics(diagnostics, {
    getCanonicalFileName: (name) => name,
    getCurrentDirectory: () => process.cwd(),
    getNewLine: () => '\n',
  }));
  for (const diagnostic of diagnostics) {
    assert.ok(diagnostic.start !== undefined);
    assert.ok(diagnostic.start >= source.indexOf('const invalidPath'));
    assert.equal(diagnostic.category, ts.DiagnosticCategory.Error);
  }
});
