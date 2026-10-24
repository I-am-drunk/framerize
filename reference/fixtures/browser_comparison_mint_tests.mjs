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

async function fixture(change = () => {}, token = PRIVATE, sdk = null) {
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
  let expression = literal;
  if (sdk) {
    const source = `let state; const logger = {};
      export function set(value) { state = value; }
      export function i() { return logger; }
      export function h() { if (state.throwRead) throw Error('private-error'); return state.records; }`;
    const loggerURL = 'data:text/javascript,' + encodeURIComponent(source).replaceAll("'", '%27') + '#' + ++sequence;
    const logger = await import(loggerURL);
    logger.set(sdk); sdk.logger = logger.i();
    // Replace only the external module address with an inert import port.
    const address = 'https://app.framerstatic.com/chunk-WBI6GGAG.mjs';
    assert.equal(literal.split(address).length, 2);
    expression = literal.replace(address, loggerURL);
  }
  const mint = new Function('location', 'localStorage', 'TextEncoder',
    'return (' + expression + ');')(location, localStorage, TextEncoder);
  return { state, run: () => mint({ url, entry: 'b', device: DEVICE,
    page_url: PAGE, observe_sdk: !!sdk }) };
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

test('SDK observations count only new matching logger records and fixed tags', async () => {
  const sdk = { records: [{ id: 4, logger: null, parts: [PRIVATE] }] };
  const value = await fixture(() => {
    sdk.records.push({ id: 5, logger: {}, parts: ['Using Castle v2 token'] },
      { id: 6, logger: sdk.logger, parts: ['Using Castle v3 token'] },
      { id: 7, logger: sdk.logger, parts: ['Using Castle v2 token'] },
      { id: 8, logger: sdk.logger, parts: [new Error(PRIVATE), PRIVATE,
        { castleVersion: 'v3', castlePhase: 'token generation', castleFailure: 'error' }] });
  }, PRIVATE, sdk);
  const result = await value.run();
  assert.equal(result.token, PRIVATE);
  assert.equal(value.state.mints, 1);
  assert.equal(result.sdk.complete, true);
  assert.equal(result.sdk.v3, 1);
  assert.equal(result.sdk.v2, 1);
  assert.equal(result.sdk.token_generation_error, 1);
  assert.equal(Object.keys(result.sdk).length, 12);
  assert.equal(JSON.stringify(result.sdk).includes(PRIVATE), false);
});

test('SDK buffer eviction keeps partial counts explicitly incomplete', async () => {
  const sdk = { records: [{ id: 4, logger: null, parts: [] }] };
  const value = await fixture(() => {
    sdk.records = [{ id: 8, logger: sdk.logger, parts: ['Using Castle v3 token'] }];
  }, PRIVATE, sdk);
  const result = await value.run();
  assert.equal(result.sdk.complete, false);
  assert.equal(result.sdk.v3, 1);
  assert.equal(result.token, PRIVATE);
});

test('malformed SDK record IDs leave diagnostics unknown without another mint', async () => {
  const sdk = { records: [] };
  const value = await fixture(() => {
    sdk.records = [{ id: 1, logger: sdk.logger, parts: [] }, { id: 1, parts: [] }];
  }, PRIVATE, sdk);
  const result = await value.run();
  assert.equal(result.sdk, null);
  assert.equal(result.token, PRIVATE);
  assert.equal(value.state.mints, 1);
});

test('unavailable SDK logger leaves the token result unchanged', async () => {
  const value = await fixture(() => {}, PRIVATE, { records: [], throwRead: true });
  const result = await value.run();
  assert.equal(result.sdk, null);
  assert.equal(result.token, PRIVATE);
  assert.equal(value.state.mints, 1);
});

test('post-mint refusal retains its stage and bounded SDK observation', async () => {
  const sdk = { records: [] };
  const value = await fixture(state => {
    state.page = 'https://framer.com/projects/other';
    sdk.records.push({ id: 0, logger: sdk.logger, parts: ['No Castle token available'] });
  }, PRIVATE, sdk);
  const result = await value.run();
  assert.equal(result.failure_stage, 'post_page');
  assert.equal(result.sdk.missing, 1);
  assert.equal(Object.hasOwn(result, 'token'), false);
});

test('oversized SDK buffers are not exported or counted', async () => {
  const sdk = { records: [] };
  const value = await fixture(() => {
    sdk.records = Array.from({ length: 1001 }, (_, id) => ({
      id, logger: sdk.logger, parts: ['Using Castle v3 token'],
    }));
  }, PRIVATE, sdk);
  const result = await value.run();
  assert.equal(result.sdk, null);
  assert.equal(result.token, PRIVATE);
});

test('malformed initial SDK history cannot claim complete coverage', async () => {
  const sdk = { records: [{ id: 4, parts: [] }, { id: 3, parts: [] }] };
  const value = await fixture(() => {
    sdk.records = [{ id: 4, logger: sdk.logger, parts: ['Using Castle v3 token'] }];
  }, PRIVATE, sdk);
  const result = await value.run();
  assert.equal(result.sdk, null);
  assert.equal(result.token, PRIVATE);
  assert.equal(value.state.mints, 1);
});

test('all allowed failure tags are counted without serializing errors or extras', async () => {
  const sdk = { records: [] };
  const value = await fixture(() => {
    for (const phase of ['module load', 'configuration', 'token generation', 'SDK'])
      for (const failure of ['error', 'timeout']) sdk.records.push({
        id: sdk.records.length, logger: sdk.logger, parts: [
          { toJSON() { throw Error(PRIVATE); } }, PRIVATE,
          { castleVersion: 'v3', castlePhase: phase, castleFailure: failure }] });
  }, PRIVATE, sdk);
  const result = await value.run();
  assert.equal(result.sdk.complete, true);
  for (const phase of ['module_load', 'configuration', 'token_generation', 'sdk'])
    for (const failure of ['error', 'timeout']) assert.equal(result.sdk[phase + '_' + failure], 1);
  assert.equal(JSON.stringify(result.sdk).includes(PRIVATE), false);
});
