import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const source = new URL('../src/compatibility/versions.ts', import.meta.url);
const compiled = await build({
  entryPoints: [fileURLToPath(source)], bundle: true,
  platform: 'node', format: 'esm', write: false,
});
const moduleUrl = 'data:text/javascript;base64,'
  + Buffer.from(compiled.outputFiles[0].contents).toString('base64');
const { fetchText } = await import(moduleUrl);
const endpoint = 'https://release.invalid/latest';

function responseFixture(context, chunks, status = 200) {
  const state = { pulled: 0, cancelled: 0 };
  const body = new ReadableStream({
    pull(controller) {
      if (state.pulled === chunks.length) return controller.close();
      controller.enqueue(chunks[state.pulled++]);
    },
    cancel() { state.cancelled++; },
  }, { highWaterMark: 0 });
  context.mock.method(globalThis, 'fetch',
    async () => new Response(body, { status }));
  return state;
}

test('reads a split UTF-8 response at the 4096-byte limit', async context => {
  const bytes = new TextEncoder().encode('a'.repeat(4094) + 'é');
  const state = responseFixture(context, [
    bytes.subarray(0, 4095), bytes.subarray(4095),
  ]);
  assert.equal(await fetchText(endpoint, 1000), 'a'.repeat(4094) + 'é');
  assert.deepEqual(state, { pulled: 2, cancelled: 0 });
});

test('cancels an oversized body before reading its tail', async context => {
  const state = responseFixture(context, [
    new Uint8Array(4097), new Uint8Array(100), new Uint8Array(100),
  ]);
  assert.equal(await fetchText(endpoint, 1000), null);
  assert.deepEqual(state, { pulled: 1, cancelled: 1 });
});

test('cancels a rejected HTTP body without reading it', async context => {
  const state = responseFixture(context, [new Uint8Array(100)], 503);
  assert.equal(await fetchText(endpoint, 1000), null);
  assert.deepEqual(state, { pulled: 0, cancelled: 1 });
});
