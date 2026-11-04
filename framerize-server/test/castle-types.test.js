import assert from 'node:assert/strict';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import ts from 'typescript';

function diagnostics(source) {
  const root = fileURLToPath(new URL('../../', import.meta.url));
  const entry = fileURLToPath(new URL('./castle-consumer.mts', import.meta.url));
  const config = ts.readConfigFile(`${root}/tsconfig.base.json`, ts.sys.readFile);
  const { options } = ts.convertCompilerOptionsFromJson(config.config.compilerOptions, root);
  const host = ts.createCompilerHost(options);
  const getSourceFile = host.getSourceFile.bind(host);
  host.getSourceFile = (name, ...args) => resolve(name) === resolve(entry)
    ? ts.createSourceFile(name, source, ts.ScriptTarget.ES2022, true)
    : getSourceFile(name, ...args);
  const program = ts.createProgram([entry], { ...options, noEmit: true }, host);
  return ts.getPreEmitDiagnostics(program).map(item => ts.flattenDiagnosticMessageText(item.messageText, '\n'));
}

test('declarations preserve minter injection and typed errors', () => {
  assert.deepEqual(diagnostics(`
    import { CastleError, createMinter, resolveOptions, encode, encodeFrame,
      runCollectionWorker, privateEnvironment, installCollectionGuard } from '../dist/castle.js';
    import type { Collection } from '../dist/castle.js';
    const collect = async (): Promise<Collection> => ({ text: 'fixture', cuid: null, replayed: 0, traceEvents: 0 });
    createMinter(resolveOptions(), { collect, encode });
    const error: CastleError = new CastleError('encoder');
    const code: string = error.code;
    const token: string = encodeFrame(new Uint8Array());
    const result: Promise<Collection> = runCollectionWorker(new URL('file:///fixture'), {}, '/tmp', new AbortController().signal, 1000);
    const environment: NodeJS.ProcessEnv = privateEnvironment('/tmp');
    installCollectionGuard(new Map<string, Buffer>());
  `), []);
});

test('declarations reject invalid public boundary inputs', () => {
  const errors = diagnostics(`
    import { CastleError, encode, encodeFrame, runCollectionWorker,
      privateEnvironment, installCollectionGuard } from '../dist/castle.js';
    const signal = new AbortController().signal;
    new CastleError('unrecognized');
    encodeFrame('text');
    encode(undefined, [], 'text', signal, '/tmp');
    runCollectionWorker('worker', {}, '/tmp', signal, 1000);
    privateEnvironment(1);
    installCollectionGuard(new Map<string, string>());
  `);
  assert.equal(errors.length, 6, errors.join('\n'));
  assert.ok(errors.every(message => message.includes('not assignable')));
});
