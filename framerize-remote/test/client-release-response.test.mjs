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

test('counts bytes across chunks before retaining more data', async context => {
  const state = responseFixture(context, [
    new Uint8Array(2048), new Uint8Array(2048),
    new Uint8Array(1), new Uint8Array(100),
  ]);
  assert.equal(await fetchText(endpoint, 1000), null);
  assert.deepEqual(state, { pulled: 3, cancelled: 1 });
});

test('the response limit counts UTF-8 bytes, not characters', async context => {
  const state = responseFixture(context, [
    new TextEncoder().encode('é'.repeat(2049)),
  ]);
  assert.equal(await fetchText(endpoint, 1000), null);
  assert.deepEqual(state, { pulled: 1, cancelled: 1 });
});

test('decodes text with the same BOM handling as fetch', async context => {
  responseFixture(context, [new TextEncoder().encode('\uFEFF2.1.300\n')]);
  assert.equal(await fetchText(endpoint, 1000), '2.1.300\n');
});

test('an absent response body remains empty text', async context => {
  context.mock.method(globalThis, 'fetch',
    async () => new Response(null, { status: 204 }));
  assert.equal(await fetchText(endpoint, 1000), '');
});

test('a failed cancellation still releases the reader', async context => {
  const body = new ReadableStream({
    pull(controller) { controller.enqueue(new Uint8Array(4097)); },
    cancel() { throw new Error('synthetic cancellation failure'); },
  }, { highWaterMark: 0 });
  context.mock.method(globalThis, 'fetch', async () => new Response(body));
  assert.equal(await fetchText(endpoint, 1000), null);
  assert.equal(body.locked, false);
});

test('a failed read returns unknown and releases the reader', async context => {
  const body = new ReadableStream({
    pull(controller) { controller.error(new Error('synthetic read failure')); },
  });
  context.mock.method(globalThis, 'fetch', async () => new Response(body));
  assert.equal(await fetchText(endpoint, 1000), null);
  assert.equal(body.locked, false);
});
