import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../scripts/validation/completion.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { inspectCompletion } = await import(`data:text/javascript;base64,${
  Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const chunk = (delta, finish_reason = null) =>
  `data: ${JSON.stringify({ choices: [{ index: 0, delta, finish_reason }] })}\n\n`;
const complete = chunk({ content: 'OK' }) + chunk({}, 'stop') + 'data: [DONE]\n\n';

for (const field of ['data:', 'data: ', 'data', 'data:\ndata:']) {
  test(`an error event with ${JSON.stringify(field)} is dispatched`, () => {
    const frame = `event: error\n${field}\n\n`;
    for (const text of [frame + complete, complete + frame]) {
      assert.throws(() => inspectCompletion(Buffer.from(text)), { message: 'Error event' });
    }
  });
}

test('an empty message event before completion is invalid JSON', () => {
  assert.throws(() => inspectCompletion(Buffer.from('data:\n\n' + complete)), SyntaxError);
});

test('an empty message event after completion is still extra data', () => {
  assert.throws(() => inspectCompletion(Buffer.from(complete + 'data:\n\n')),
    { message: 'Data after completion' });
});
