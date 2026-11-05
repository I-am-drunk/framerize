import assert from 'node:assert/strict';
import { copyFile, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { getEventListeners } from 'node:events';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import test from 'node:test';
import { createCastleMinter } from '../dist/castle.js';

test('a standalone copy needs no adjacent assets or source repository', async context => {
  const directory = await mkdtemp(join(tmpdir(), 'castle-bundle-'));
  context.after(() => rm(directory, { recursive: true, force: true }));
  const entry = join(directory, 'castle.mjs');
  await copyFile(new URL('../dist/castle.js', import.meta.url), entry);
  const library = await import(pathToFileURL(entry).href);
  const minter = library.createCastleMinter();
  context.after(() => minter.close());
  const result = await minter.mint({ cuid: 'standalone-fixture' });
  assert.equal(result.cuid, 'standalone-fixture');
  assert.equal(result.traceEvents, 198);
  assert.ok(result.replayed > 0);
  assert.equal(result.encoding, 'wire-token');
  assert.match(result.token, /^cNz8rPYH\|[A-Za-z0-9+/]+={0,2}$/);
  assert.equal(result.tokenLength, result.token.length);
  assert.ok(Object.isFrozen(result));
  await minter.close();
  await assert.rejects(minter.mint(), { code: 'closed' });
});

for (const [name, text, expected] of [
  ['empty', '{"events":[]}', null],
  ['malformed', '{"events":[null]}', 'collection'],
  ['oversized', ' '.repeat(1024 * 1024 + 1), 'collection'],
  ['realm-state', '{"events":[]}', 'collection'],
]) {
  test(`explicit ${name} trace preserves collection validation`, async context => {
    const directory = await mkdtemp(join(tmpdir(), 'castle-trace-'));
    context.after(() => rm(directory, { recursive: true, force: true }));
    const tracePath = join(directory, `${name}.json`);
    await writeFile(tracePath, text);
    const minter = createCastleMinter({ tracePath });
    context.after(() => minter.close());
    if (expected) await assert.rejects(minter.mint(), { code: expected });
    else {
      const result = await minter.mint();
      assert.equal(result.traceEvents, 0);
      assert.equal(result.replayed, 0);
    }
    assert.equal(minter.stats().busy, false);
    assert.equal(minter.stats().failures, expected ? 1 : 0);
    await minter.close();
    assert.equal(await readFile(tracePath, 'utf8'), text);
  });
}

test('public minter rejects pre-abort and drains immediate close', async () => {
  const controller = new AbortController();
  controller.abort(new Error('synthetic-secret'));
  const minter = createCastleMinter();
  try {
    await assert.rejects(minter.mint({ signal: controller.signal }), { code: 'aborted' });
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    let settled = false;
    const pending = minter.mint().finally(() => { settled = true; });
    const rejection = assert.rejects(pending, { code: 'aborted' });
    await minter.close();
    assert.equal(settled, true);
    await rejection;
    assert.deepEqual(minter.stats(), { mints: 0, failures: 2, busy: false, traceEvents: 0 });
  } finally { await minter.close(); }
});
