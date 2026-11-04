import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/config.ts', import.meta.url))],
  bundle: true,
  platform: 'node',
  format: 'esm',
  write: false,
});
const { ConfigValidationError, parsePersisted } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

const base = {
  remote: 'http://127.0.0.1:1',
  serverToken: { kind: 'env', name: 'SERVER' },
  localToken: { kind: 'env', name: 'LOCAL' },
};

const cases = [
  ['inference_invalid', { ...base, inferenceTimeoutMs: 0 }],
  ['listen_invalid', { ...base, host: 'bad/host' }],
  ['listen_invalid', { ...base, port: 70000 }],
  ['server_reference', { ...base, serverToken: { kind: 'env', name: '1bad' } }],
  ['local_reference', { ...base, localToken: { kind: 'file', path: '' } }],
];

for (const [code, value] of cases) {
  test(`parsePersisted reports ${code} for invalid saved fields`, () => {
    assert.throws(() => parsePersisted(value), error => {
      assert.ok(error instanceof ConfigValidationError);
      assert.equal(error.code, code);
      return true;
    });
  });
}
