import test from 'node:test';
import assert from 'node:assert/strict';
import { LegacyV2Operation } from '../../scripts/offline_worker/v2_operation.mjs';
import { createLegacyV2Minter } from '../../scripts/offline_worker/v2_runtime.mjs';
import { validateInput } from '../../scripts/offline_worker/protocol.mjs';

test('resident_v2_preserves_the_exact_captured_login_url', async () => {
  const h = harness(), value = input();
  value.target.url = 'https://framer.com/login?fixture=1';
  assert.throws(() => validateInput(value));
  assert.throws(() => new LegacyV2Operation(value, h.runtime));
  const result = await new LegacyV2Operation(value, h.runtime, true).mint();
  assert.equal(result.metadata.status, 'passed');
  assert.equal(h.state.url, value.target.url);
  assert.equal(h.state.calls, 1);
});

test('resident_login_exception_keeps_origin_path_and_device_guards', () => {
  for (const url of ['https://other.invalid/login', 'http://framer.com/login',
    'https://framer.com/login/other', 'https://framer.com/projects/other']) {
    const value = input(); value.target.url = url;
    assert.throws(() => new LegacyV2Operation(value, harness().runtime, true));
  }
  const value = input(); value.target.url = 'https://framer.com/login';
  delete value.device; value.version = 1;
  assert.throws(() => new LegacyV2Operation(value, harness().runtime, true));
});

const input = () => ({ version: 2, budget_ms: 1000, device: { id: 'a'.repeat(32) },
  target: { project_id: 'offline-fixture', path: '/ai/v3/responses/',
    url: 'https://framer.com/projects/offline-fixture' } });
test('production_constructor_is_inert', () => {
  assert.ok(createLegacyV2Minter(input()) instanceof LegacyV2Operation);
});
function harness() {
  const state = { verify: 0, admit: 0, closed: 0, removed: 0, calls: 0, time: 0 };
  const values = new Map(); let cookie = '';
  const window = { close() { state.closed++; }, localStorage: { get length() { return values.size; },
    getItem: key => values.get(key), setItem: (key, value) => values.set(key, value) },
    document: { get cookie() { return cookie; },
      set cookie(value) { cookie = value.split(';')[0]; } } };
  const instance = { createRequestToken(...args) {
    assert.equal(this, instance); assert.deepEqual(args, []);
    state.calls++; state.time += 5; return Promise.resolve('fixture-token');
  } };
  const runtime = { now: () => state.time, verify() { state.verify++; },
    admit() { state.admit++; }, observe() { return () => state.removed++; },
    async createWindow(url) { state.url = url;
      return { window }; },
    async configure(_window, report) { report.configure_calls++; return instance; },
    async settle(invoke) {
      try { const value = await invoke(); return { outcome: 'fulfilled',
        nonempty_string: typeof value === 'string' && value.length > 0,
        late_settlements: 0 }; }
      catch { return { outcome: 'rejected', nonempty_string: false, late_settlements: 0 }; }
    }, async drain() {} };
  return { state, runtime, window, instance,
    run: () => new LegacyV2Operation(input(), runtime).mint() };
}

function afterMint(h, change) {
  const invoke = h.instance.createRequestToken;
  h.instance.createRequestToken = function() {
    const result = invoke.call(this);
    change();
    return result;
  };
}

test('exact_receiver_one_call_saved_identity_and_private_token', async () => {
  const h = harness(); const operation = new LegacyV2Operation(input(), h.runtime);
  const r = await operation.mint();
  assert.equal(r.metadata.status, 'passed');
  assert.equal(r.metadata.device_failure, null);
  assert.equal(h.state.url, input().target.url);
  assert.equal(h.window.localStorage.getItem('__cuid'), input().device.id);
  assert.equal(h.state.calls, 1); assert.equal(h.state.closed, 1);
  assert.equal(h.state.removed, 1); assert.equal(h.state.verify, 2);
  assert.equal(r.metadata.token_method_calls, 1);
  assert.equal(r.metadata.selection, 'configured-api');
  assert.equal(r.metadata.sdk, 'framer-embedded-v2');
  assert.throws(() => JSON.stringify(r.token), { message: 'private-token' });
  assert.equal(r.token.take(), 'fixture-token');
  assert.throws(() => r.token.take(), { message: 'token-consumed' });
  await assert.rejects(operation.mint(), { message: 'mint-already-used' });
  assert.equal(h.state.calls, 1);
});

test('missing_saved_identity_and_foreign_target_refuse_before_runtime', () => {
  const h = harness(); const { device, ...value } = input();
  assert.throws(() => new LegacyV2Operation({ ...value, version: 1 }, h.runtime));
  assert.throws(() => new LegacyV2Operation({ ...input(), extra: true }, h.runtime));
  const bad = input(); bad.target.url = 'https://framer.com/projects/other';
  assert.throws(() => new LegacyV2Operation(bad, h.runtime));
  assert.equal(h.state.admit, 0); assert.equal(h.state.verify, 0);
});
async function refused(h, phase, calls = 1) {
  const r = await h.run();
  assert.equal(r.token, undefined); assert.equal(r.minted_at, undefined);
  assert.equal(r.metadata.status, 'failed');
  assert.equal(r.metadata.failure_phase, phase);
  assert.equal(h.state.calls, calls); assert.equal(h.state.closed, 1);
  assert.equal(h.state.removed, 1);
  assert.equal(JSON.stringify(r.metadata).includes('private-error'), false);
  assert.equal(JSON.stringify(r.metadata).includes(input().device.id), false);
  return r;
}

test('configuration_failure_still_closes_and_never_mints', async () => {
  const h = harness();
  h.runtime.configure = async () => { throw new Error('private-error'); };
  const r = await refused(h, 'configure', 0);
  assert.equal(r.metadata.device_failure, null);
});
test('configuration_device_drift_refuses_before_mint', async () => {
  const h = harness(); const configure = h.runtime.configure;
  h.runtime.configure = async (...args) => {
    const result = await configure(...args);
    h.window.localStorage.setItem('__cuid', 'different'); return result;
  };
  const r = await refused(h, 'configure', 0);
  assert.equal(r.metadata.device_failure, 'local_value');
});
test('mint_device_drift_discards_result', async () => {
  const h = harness(); const invoke = h.instance.createRequestToken;
  h.instance.createRequestToken = function() {
    const result = invoke.call(this);
    h.window.document.cookie = '__cuid=different'; return result;
  };
  const r = await refused(h, 'device-storage');
  assert.equal(r.metadata.device_failure, 'cookie_value');
});
test('identical_device_cookie_rows_refuse_without_local_value_drift', async () => {
  const h = harness(), pair = '__cuid=' + input().device.id;
  afterMint(h, () => Object.defineProperty(h.window.document, 'cookie', {
    get: () => pair + '; ' + pair,
  }));
  const r = await refused(h, 'device-storage');
  assert.equal(r.metadata.device_failure, 'cookie_count');
  assert.equal(h.window.localStorage.getItem('__cuid'), input().device.id);
});

test('missing_device_cookie_reports_count_without_local_value_drift', async () => {
  const h = harness();
  afterMint(h, () => { h.window.document.cookie = 'other=fixture'; });
  const r = await refused(h, 'device-storage');
  assert.equal(r.metadata.device_failure, 'cookie_count');
  assert.equal(h.window.localStorage.getItem('__cuid'), input().device.id);
});

test('device_getter_failures_report_only_fixed_categories', async () => {
  for (const kind of ['cookie_read', 'local_read']) {
    const h = harness(), fail = () => { throw new Error('private-error'); };
    afterMint(h, () => {
      if (kind === 'cookie_read')
        Object.defineProperty(h.window.document, 'cookie', { get: fail });
      else h.window.localStorage.getItem = fail;
    });
    const r = await refused(h, 'device-storage');
    assert.equal(r.metadata.device_failure, kind);
  }
});

test('foreign_error_properties_cannot_forge_device_diagnostics', async () => {
  const h = harness();
  h.runtime.configure = async () => {
    throw Object.assign(new Error('device-storage-changed'), {
      kind: 'cookie_value', device_failure: 'cookie_value',
    });
  };
  const r = await refused(h, 'configure', 0);
  assert.equal(r.metadata.device_failure, null);
});

test('local_value_mismatch_keeps_priority_over_cookie_count', async () => {
  const h = harness(), pair = '__cuid=' + input().device.id;
  afterMint(h, () => {
    h.window.localStorage.setItem('__cuid', 'different');
    Object.defineProperty(h.window.document, 'cookie', { get: () => pair + '; ' + pair });
  });
  const r = await refused(h, 'device-storage');
  assert.equal(r.metadata.device_failure, 'local_value');
});

test('non_thenable_return_cannot_pass', async () => {
  const h = harness();
  h.instance.createRequestToken = () => { h.state.calls++; return 'fixture-token'; };
  await refused(h, 'mint');
});
test('close_failure_discards_result', async () => {
  const h = harness(); const create = h.runtime.createWindow;
  h.runtime.createWindow = async (...args) => {
    const dom = await create(...args);
    dom.window.close = () => { h.state.closed++; throw new Error('private-error'); };
    return dom;
  };
  await refused(h, 'cleanup');
});
test('changed_source_discards_result_after_close', async () => {
  const h = harness();
  h.runtime.verify = () => { if (++h.state.verify === 2) throw new Error('private-error'); };
  await refused(h, 'input-identity');
});
test('late_method_result_cannot_pass_the_remaining_budget', async () => {
  const h = harness(); const invoke = h.instance.createRequestToken;
  h.instance.createRequestToken = function() {
    const result = invoke.call(this); h.state.time = 1001; return result;
  };
  await refused(h, 'mint');
});
test('cleanup_time_counts_toward_total_budget', async () => {
  const h = harness(); h.runtime.drain = async () => { h.state.time = 1001; };
  await refused(h, 'deadline');
});
test('async_fault_discards_otherwise_valid_token', async () => {
  const h = harness();
  h.runtime.observe = report => {
    report.async_errors++; return () => h.state.removed++;
  };
  await refused(h, 'cleanup');
});
test('oversized_or_invalid_utf8_token_is_not_exposed', async () => {
  for (const value of ['x'.repeat(16385), '\ud800']) {
    const h = harness();
    h.instance.createRequestToken = () => { h.state.calls++; return Promise.resolve(value); };
    await refused(h, 'token-shape');
  }
});
test('timeout_then_late_token_never_becomes_a_handoff', async () => {
  const h = harness(); let deliver;
  h.instance.createRequestToken = () => {
    h.state.calls++; return new Promise(resolve => { deliver = resolve; });
  };
  h.runtime.settle = async invoke => {
    const result = { outcome: 'timeout', nonempty_string: false, late_settlements: 0 };
    invoke().then(() => { result.late_settlements++; });
    return result;
  };
  h.runtime.drain = async () => {
    deliver('late-private-token');
    await new Promise(resolve => setImmediate(resolve));
  };
  const r = await refused(h, 'mint');
  assert.equal(r.metadata.late_settlements, 1);
  assert.equal(JSON.stringify(r).includes('late-private-token'), false);
});
