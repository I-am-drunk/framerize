import assert from 'node:assert/strict';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/frmCli.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { runFrm } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

for (const args of [
  ['up', 'http://127.0.0.1:1', 'unexpected'],
  ['up', '--json', 'http://127.0.0.1:1', 'unexpected'],
]) test(`frm ${args.join(' ')} rejects extra arguments before reading a profile`, async t => {
  const previous = process.env.FRM_HOME;
  process.env.FRM_HOME = 'invalid-relative-home';
  t.after(() => {
    if (previous === undefined) delete process.env.FRM_HOME;
    else process.env.FRM_HOME = previous;
  });
  const io = { stdout() { assert.fail('Unexpected output'); },
    stderr() { assert.fail('Unexpected output'); },
    async ask() { assert.fail('Invalid arguments must not prompt'); } };
  await assert.rejects(runFrm(args, io, { program: ['/synthetic/frm'] }),
    { code: 'command_arguments' });
});
