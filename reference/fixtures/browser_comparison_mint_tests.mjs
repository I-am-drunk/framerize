// Exact-expression inert fixtures. UNRUN by the author. No SDK or network.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const source = readFileSync(new URL('../../scripts/contextual_authorization.py', import.meta.url), 'utf8');
const marker = 'COMPARISON_MINT = """';
const start = source.indexOf(marker);
assert.ok(start >= 0);
assert.equal(source.indexOf(marker, start + marker.length), -1);
const end = source.indexOf('"""', start + marker.length);
assert.ok(end > start);
const literal = source.slice(start + marker.length, end);
assert.equal(literal.includes('\\'), false);
const PAGE = 'https://framer.com/projects/fixture-project';
const DEVICE = 'fixture-device';
const PRIVATE = 'private-fixture-material';
let sequence = 0;

async function fixture(change = () => {}, token = PRIVATE) {
  const state = { page: PAGE, device: DEVICE, mints: 0, storageReads: 0,
    throwPage: false, throwDevice: false };
  const location = { get href() {
    if (state.throwPage) throw new Error(PRIVATE);
    return state.page;
  } };
  const localStorage = { getItem(key) {
    assert.equal(key, '__cuid');
    state.storageReads++;
    if (state.throwDevice) throw new Error(PRIVATE);
    return state.device;
  } };
  const url = 'data:text/javascript,' + encodeURIComponent(
    'export let b; export function set(value) { b = value; }') + '#' + ++sequence;
  const wrapper = await import(url);
  wrapper.set(async () => { state.mints++; change(state); return token; });
  const mint = new Function('location', 'localStorage', 'TextEncoder',
    'return (' + literal + ');')(location, localStorage, TextEncoder);
  return { state, run: () => mint({ url, entry: 'b', device: DEVICE, page_url: PAGE }) };
}

async function refused(value, stage) {
  assert.deepEqual(await value.run(), { failure_stage: stage });
  assert.equal(value.state.mints, 1);
}

test('post-mint page mismatch is fixed and skips the later device read', async () => {
  const value = await fixture(state => {
    state.page = 'https://framer.com/projects/other-fixture';
    state.throwDevice = true;
  });
  await refused(value, 'post_page');
  assert.equal(value.state.storageReads, 1);
});

test('post-mint device mismatch returns only its fixed stage', async () => {
  const value = await fixture(state => { state.device = 'other-fixture-device'; });
  await refused(value, 'post_device');
  assert.equal(value.state.storageReads, 2);
});

test('post-mint page getter exception keeps post_context without error details', async () => {
  const value = await fixture(state => { state.throwPage = true; });
  await refused(value, 'post_context');
  assert.equal(value.state.storageReads, 1);
});

test('post-mint storage exception keeps post_context without error details', async () => {
  const value = await fixture(state => { state.throwDevice = true; });
  await refused(value, 'post_context');
  assert.equal(value.state.storageReads, 2);
});

test('unchanged post-mint context retains the exact success envelope', async () => {
  const value = await fixture();
  assert.deepEqual(await value.run(), { before: PAGE, after: PAGE, token: PRIVATE });
  assert.equal(value.state.mints, 1);
  assert.equal(value.state.storageReads, 2);
});

test('unchanged context still rejects invalid or oversized token encoding', async () => {
  for (const token of [null, '', '\ud800', 'x'.repeat(16385), 'é'.repeat(8193)]) {
    await refused(await fixture(() => {}, token), 'invalid_token');
  }
});
