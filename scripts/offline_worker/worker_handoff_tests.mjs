// Actual entrypoint with inert module/pipe substitutes. No SDK or child process.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import vm from 'node:vm';

const source = await readFile(new URL('./worker.mjs', import.meta.url), 'utf8');
const report = (version = 1) => ({ version, status: 'passed', failure_phase: null,
  target_validated: true, account_state_loaded: false, wrapper_calls: 1,
  branch: { v3: 1, v2: 0, none: 0, errors: 0 }, dynamic_imports: 1,
  v3_evaluated: true, window_closed: true, inspector_closed: true,
  sources_unchanged: true, async_errors: 0, diagnostics: 2, late_settlements: 0,
  method_ms: 3, elapsed_ms: 5, collection_to_handoff_ms: 0, collection_handoff: false,
  ...(version === 2 ? { device_storage_seeded: true, device_storage_unchanged: true } : {}),
});

async function run({ fail, times = [10, 20], version = 1, failedMint = false } = {}) {
  const original = report(version), metadata = structuredClone(original);
  if (failedMint) Object.assign(metadata, { status: 'failed', failure_phase: 'cleanup' });
  const calls = [], closed = [], written = [], exits = [];
  let takes = 0, writes = 0;
  const step = name => { calls.push(name); if (fail === name) throw Error('private-fixture-marker'); };
  const fs = { closeSync(fd) { closed.push(fd); }, writeSync(fd, bytes, offset, length) {
    assert.equal(fd, 5); written.push(Buffer.from(bytes.subarray(offset, offset + length)));
    return length;
  } };
  const context = vm.createContext({ Buffer, process: { exit(code) { exits.push(code); } } });
  const module = exports => new vm.SyntheticModule(Object.keys(exports), function () {
    for (const [name, value] of Object.entries(exports)) this.setExport(name, value);
  }, { context });
  const imports = {
    'node:fs': module({ default: fs }),
    'node:perf_hooks': module({ performance: { now() { return times.shift(); } } }),
    './offline_mint.mjs': module({ OfflineCastleMinter: class {
      constructor() { step('input'); }
      async mint() { step('mint'); return { metadata, collected_at: 0,
        collection: failedMint ? undefined : { take() { takes++; return 'synthetic-private-collection'; } } }; }
    } }),
    './protocol.mjs': module({ readInput: () => ({}), verifyDescriptors: () => step('descriptors') }),
    './sdk_loader.mjs': module({ verifyInputs: () => step('verify') }),
    './collection.mjs': module({ writeCollection(fd, value) {
      assert.equal(fd, 4);
      try { writes++; assert.equal(value, 'synthetic-private-collection'); step('write'); }
      finally { closed.push(fd); }
    } }),
  };
  const entry = new vm.SourceTextModule(source, { context });
  await entry.link(name => { assert.ok(imports[name]); return imports[name]; });
  await entry.evaluate();
  const output = Buffer.concat(written).toString('utf8');
  assert.equal(output.includes('private'), false);
  return { metadata: output ? JSON.parse(output) : null, original, calls, closed, exits, takes, writes };
}

function refused(result, attempted, completed, age) {
  assert.deepEqual(result.metadata, { ...result.original, status: 'failed',
    failure_phase: 'handoff', collection_pipe_write_attempted: attempted,
    collection_pipe_write_completed: completed, collection_to_handoff_ms: age });
  assert.deepEqual(result.exits, [2]);
  assert.deepEqual(result.closed, [4, 5]);
  assert.equal(result.takes, attempted ? 1 : 0);
  assert.equal(result.writes, attempted ? 1 : 0);
}

test('early_errors_remain_minimal_and_untrusted_descriptors_emit_nothing', async () => {
  for (const fail of ['descriptors', 'input', 'mint']) {
    const result = await run({ fail });
    assert.deepEqual(result.metadata, fail === 'descriptors' ? null
      : { version: 1, status: 'failed', failure_phase: fail });
    assert.deepEqual(result.exits, [2]);
    assert.deepEqual(result.closed, fail === 'descriptors' ? [4] : [4, 5]);
    assert.equal(result.takes, 0); assert.equal(result.writes, 0);
  }
});

test('failed_minter_report_is_preserved_without_handoff_claims', async () => {
  for (const version of [1, 2]) {
    const result = await run({ version, failedMint: true });
    assert.deepEqual(result.metadata, { ...result.original, status: 'failed', failure_phase: 'cleanup' });
    assert.deepEqual(result.exits, [2]); assert.deepEqual(result.closed, [4, 5]);
    assert.equal(result.calls.includes('verify'), false);
    assert.equal(result.takes, 0); assert.equal(result.writes, 0);
  }
});

test('verification_refusal_retains_completed_facts', async () => {
  for (const version of [1, 2]) {
    refused(await run({ version, fail: 'verify' }), false, false, null);
  }
});

test('stale_before_write_retains_facts_without_consuming_token', async () => {
  for (const version of [1, 2]) refused(await run({ version, times: [2001] }), false, false, 2001);
});

test('writer_failure_is_attempted_but_not_completed_or_age_qualified', async () => {
  for (const version of [1, 2]) refused(await run({ version, fail: 'write' }), true, false, 10);
});

test('stale_after_write_is_completed_but_not_age_qualified', async () => {
  for (const version of [1, 2]) refused(await run({ version, times: [10, 2001] }), true, true, 2001);
});

test('invalid_age_is_unknown_and_never_qualified', async () => {
  for (const age of [NaN, Infinity, -1]) {
    refused(await run({ times: [age] }), false, false, null);
    refused(await run({ times: [10, age] }), true, true, null);
  }
});

test('successful_handoff_preserves_exact_success_schema_in_both_modes', async () => {
  for (const version of [1, 2]) {
    const result = await run({ version, times: [2000, 2000] });
    assert.deepEqual(result.metadata, { ...result.original, collection_to_handoff_ms: 2000, collection_handoff: true });
    assert.deepEqual(result.exits, [0]); assert.deepEqual(result.closed, [4, 5]);
    assert.equal(result.takes, 1); assert.equal(result.writes, 1);
  }
});
