import test from 'node:test';
import assert from 'node:assert/strict';
import { runV2Handoff } from '../../scripts/offline_worker/v2_handoff.mjs';
function harness() {
  const state = { written: 0, closed: 0, taken: 0, time: 10 };
  const metadata = { version: 1, sdk: 'framer-embedded-v2', selection: 'configured-api',
    status: 'passed', failure_phase: null, device_failure: null,
    configure_calls: 1, token_method_calls: 1, device_storage_seeded: true,
    device_storage_unchanged: true, window_closed: true, sources_unchanged: true,
    async_errors: 0, diagnostics: 19, late_settlements: 0, method_ms: 2, elapsed_ms: 5 };
  const result = { metadata, minted_at: 9,
    token: { take() { state.taken++; return 'private-fixture-token'; } } };
  const ports = { verifyDescriptors() {}, readInput() { return {}; },
    createMinter() { return { async mint() { return result; } }; },
    verify() {}, now: () => state.time,
    async writer() { return value => {
      assert.equal(value, 'private-fixture-token'); state.written++; state.closed++;
    }; },
    closeToken() { state.closed++; }, writeMetadata(value) { state.report = value; } };
  return { state, result, ports, run: () => runV2Handoff(ports) };
}
test('one_private_token_with_exact_component_facts', async () => {
  const h = harness(); assert.equal(await h.run(), 0);
  assert.equal(h.state.written, 1); assert.equal(h.state.closed, 1);
  assert.equal(h.state.taken, 1); assert.equal(h.state.report.token_handoff, true);
  assert.deepEqual(h.state.report.operation, h.result.metadata);
  assert.equal(JSON.stringify(h.state.report).includes('private-fixture-token'), false);
});
test('failed_component_never_writes_token', async () => {
  const h = harness(); h.result.metadata.status = 'failed';
  h.result.metadata.failure_phase = 'device-storage';
  h.result.metadata.device_failure = 'cookie_count';
  assert.equal(await h.run(), 2); assert.equal(h.state.written, 0);
  assert.equal(h.state.closed, 1);
  assert.equal(h.state.report.operation.device_failure, 'cookie_count');
});
test('age_before_write_refuses_without_consuming', async () => {
  const h = harness(); h.state.time = 2010;
  assert.equal(await h.run(), 2); assert.equal(h.state.taken, 0);
  assert.equal(h.state.written, 0); assert.equal(h.state.closed, 1);
});
test('age_after_write_invalidates_handoff_receipt', async () => {
  const h = harness(); const writer = h.ports.writer;
  h.ports.writer = async () => { const write = await writer();
    return value => { write(value); h.state.time = 2010; }; };
  assert.equal(await h.run(), 2); assert.equal(h.state.written, 1);
  assert.equal(h.state.report.token_handoff, false);
  assert.equal(h.state.report.failure_phase, 'handoff');
});
test('changed_input_prevents_handoff', async () => {
  const h = harness(); h.ports.verify = () => { throw new Error('private-value'); };
  assert.equal(await h.run(), 2); assert.equal(h.state.written, 0);
  assert.equal(h.state.closed, 1);
  assert.equal(JSON.stringify(h.state.report).includes('private-value'), false);
});
test('foreign_fields_cannot_leave_in_metadata', async () => {
  const h = harness(); h.result.metadata.secret = 'private-value';
  assert.equal(await h.run(), 0);
  assert.equal(JSON.stringify(h.state.report).includes('private-value'), false);
  const invalid = harness(); invalid.result.metadata.device_failure = 'private-value';
  assert.equal(await invalid.run(), 2);
  assert.equal(invalid.state.report.operation, null);
});
test('untrusted_descriptors_receive_no_metadata', async () => {
  const h = harness(); h.ports.verifyDescriptors = () => { throw new Error(); };
  assert.equal(await h.run(), 2); assert.equal(h.state.report, undefined);
  assert.equal(h.state.taken, 0); assert.equal(h.state.closed, 1);
});
test('failed_writer_or_metadata_never_reports_success', async () => {
  const h = harness();
  h.ports.writer = async () => () => { h.state.closed++; throw new Error(); };
  assert.equal(await h.run(), 2); assert.equal(h.state.closed, 1);
  assert.equal(h.state.report.token_handoff, false);
  const badMetadata = harness(); badMetadata.ports.writeMetadata = () => { throw new Error(); };
  assert.equal(await badMetadata.run(), 3);
});
