import test from 'node:test';
import assert from 'node:assert/strict';
import { validateInput, OpaqueToken } from './protocol.mjs';
import { observeBranch, exactV3, observeFailurePhase, refineFailurePhase } from './branch.mjs';
import { OfflineCastleMinter } from './offline_mint.mjs';

const input = () => ({ version: 1, budget_ms: 16000, target: {
  project_id: 'offline-fixture', path: '/ai/v3/responses/',
  url: 'https://framer.com/projects/offline-fixture',
} });
const entry = (message, level = 1) => ({ logger: { id: 'castle' }, level, parts: [message] });

const phaseCases = [
  ['module load', 'error', 'v3-module-load-error'],
  ['module load', 'timeout', 'v3-module-load-timeout'],
  ['configuration', 'error', 'v3-configuration-error'],
  ['configuration', 'timeout', 'v3-configuration-timeout'],
  ['token generation', 'error', 'v3-token-generation-error'],
  ['token generation', 'timeout', 'v3-token-generation-timeout'],
  ['SDK', 'error', 'v3-sdk-error'],
];
const phaseEntry = (phase = 'configuration', failure = 'error') => ({
  logger: { id: 'castle' }, level: 4,
  parts: [Object.create(null), { castleVersion: 'v3', castlePhase: phase, castleFailure: failure }],
});

test('wrapper_phase_tags_refine_only_supported_failed_verdicts', () => {
  for (const [phase, failure, label] of phaseCases) {
    const value = phaseEntry(phase, failure);
    for (const parts of [[value.parts[1]], value.parts, [value.parts[0], {}, value.parts[1]]]) {
      const records = [{ ...value, parts }];
      assert.equal(observeFailurePhase(records), label);
      assert.equal(refineFailurePhase('mint', label), label);
      assert.equal(refineFailurePhase('branch', label), label);
      assert.deepEqual(observeBranch(records), { v3: 0, v2: 0, none: 0, errors: 1 });
      assert.equal(exactV3(observeBranch([entry('Using Castle v3 token'), ...records])), false);
      assert.equal(refineFailurePhase(null, label), null);
    }
  }
});

test('phase_conflicts_do_not_depend_on_logger_order', () => {
  for (const [phase, failure, label] of phaseCases) {
    for (const [otherPhase, otherFailure, otherLabel] of phaseCases) {
      const actual = observeFailurePhase([
        phaseEntry(phase, failure), phaseEntry(otherPhase, otherFailure),
      ]);
      assert.equal(actual, label === otherLabel ? label : undefined);
      assert.equal(refineFailurePhase('mint', actual), label === otherLabel ? label : 'mint');
    }
  }
});

test('malformed_or_unsupported_error_tags_prevent_refinement', () => {
  const valid = phaseEntry();
  const tags = valid.parts[1];
  const invalidParts = [undefined, {}, [], [{}, tags, {}], [{}, tags, {}, {}], [{}, tags, 'private'],
    [{}, {}], [{}, { ...tags, castleVersion: 'v2' }],
    [{}, { ...tags, castleVersion: 'V3' }], [{}, { ...tags, castlePhase: 'unknown' }],
    [{}, { ...tags, castlePhase: 'SDK', castleFailure: 'timeout' }],
    [{}, { ...tags, castleFailure: 'Error' }], [{}, { ...tags, castleFailure: new String('error') }],
    [{}, Object.create(tags)], [{}, { ...tags, castlePhase: undefined }],
    [{}, { ...tags, castleFailure: null }]];
  for (const parts of invalidParts) {
    const invalid = { ...valid, parts };
    for (const records of [[valid, invalid], [invalid, valid]]) {
      assert.equal(observeFailurePhase(records), undefined);
      assert.equal(refineFailurePhase('branch', observeFailurePhase(records)), 'branch');
    }
  }
});

test('phase_reader_ignores_other_loggers_and_nonerror_parts', () => {
  let reads = 0;
  const unread = () => { reads++; throw new Error('private fixture'); };
  const foreign = { logger: { id: 'other' }, get level() { return unread(); },
    get parts() { return unread(); } };
  const warning = { logger: { id: 'castle' }, level: 3, get parts() { return unread(); } };
  assert.equal(observeFailurePhase([foreign, warning]), undefined);
  assert.equal(observeFailurePhase([foreign, warning, phaseEntry()]), 'v3-configuration-error');
  assert.equal(reads, 0);
});

test('phase_reader_never_invokes_accessors_or_uses_inherited_tags', () => {
  const locations = [r => [r, '0'], r => [r[0], 'logger'], r => [r[0].logger, 'id'],
    r => [r[0], 'level'], r => [r[0], 'parts'], r => [r[0].parts, '1'],
    r => [r[0].parts[1], 'castleVersion'], r => [r[0].parts[1], 'castlePhase'],
    r => [r[0].parts[1], 'castleFailure']];
  for (const locate of locations) {
    const records = [phaseEntry()];
    const [target, key] = locate(records);
    let reads = 0;
    Object.defineProperty(target, key, { get() { reads++; throw new Error('private fixture'); } });
    assert.equal(observeFailurePhase(records), undefined);
    assert.equal(reads, 0);
  }
  for (const key of ['castleVersion', 'castlePhase', 'castleFailure']) {
    const value = phaseEntry(), tags = value.parts[1], inherited = { [key]: tags[key] };
    delete tags[key]; Object.setPrototypeOf(tags, inherited);
    assert.equal(observeFailurePhase([value]), undefined);
  }
});

test('phase_reader_does_not_read_errors_extras_or_private_tag_fields', () => {
  const value = phaseEntry(), tags = value.parts[1];
  let reads = 0;
  const unread = () => { reads++; throw new Error('private fixture'); };
  value.parts = new Array(3);
  Object.defineProperty(value.parts, '0', { get: unread });
  Object.defineProperty(value.parts, '1', { get: unread });
  value.parts[2] = tags;
  for (const key of ['message', 'cause', 'stack', 'token', 'key', 'identifier',
    'castleTimeoutMs', 'castleDurationMs', 'toJSON', Symbol.toPrimitive])
    Object.defineProperty(tags, key, { get: unread });
  const label = observeFailurePhase([value]);
  assert.equal(label, 'v3-configuration-error');
  assert.equal(JSON.stringify({ failure_phase: refineFailurePhase('mint', label) }),
    '{"failure_phase":"v3-configuration-error"}');
  assert.equal(reads, 0);
});

test('phase_reader_refuses_malformed_inventory', () => {
  for (const records of [null, {}, Object.create([]), new Array(1001), [null],
    new Array(1), [{ logger: { id: 'castle' }, level: '4' }]])
    assert.equal(observeFailurePhase(records), undefined);
  assert.equal(observeFailurePhase([]), undefined);
});

test('phase_reader_rejects_proxies_without_running_traps', () => {
  let traps = 0;
  const trap = () => { traps++; throw new Error('private fixture'); };
  const proxy = target => new Proxy(target, { get: trap, getOwnPropertyDescriptor: trap,
    ownKeys: trap, getPrototypeOf: trap });
  const builders = [() => proxy([phaseEntry()]), () => [proxy(phaseEntry())],
    () => { const e = phaseEntry(); e.logger = proxy(e.logger); return [e]; },
    () => { const e = phaseEntry(); e.parts = proxy(e.parts); return [e]; },
    () => { const e = phaseEntry(); e.parts[1] = proxy(e.parts[1]); return [e]; }];
  for (const build of builders) assert.equal(observeFailurePhase(build()), undefined);
  const revoked = Proxy.revocable([phaseEntry()], {}); revoked.revoke();
  assert.equal(observeFailurePhase(revoked.proxy), undefined);
  assert.equal(traps, 0);
});

test('phase_refinement_preserves_success_and_other_failure_precedence', () => {
  const preserved = [null, undefined, 'descriptors', 'input', 'input-identity', 'runtime',
    'import', 'window', 'graphics', 'device-storage', 'module-link', 'inspector',
    'module-evaluate', 'dedicated-worker', 'shared-worker', 'graphics-cleanup',
    'deadline', 'token-shape', 'cleanup', 'handoff'];
  for (const phase of preserved)
    for (const [, , label] of phaseCases) assert.equal(refineFailurePhase(phase, label), phase);
  for (const phase of ['mint', 'branch'])
    for (const label of [undefined, null, 'v3-sdk-timeout', 'private', {}])
      assert.equal(refineFailurePhase(phase, label), phase);
});

test('constructor_is_inert_without_runtime_admission', () => {
  assert.ok(new OfflineCastleMinter(input()) instanceof OfflineCastleMinter);
});
test('target_is_copied_and_frozen', () => {
  const value = input(); const bound = validateInput(value); value.target.project_id = 'changed';
  assert.equal(bound.target.project_id, 'offline-fixture'); assert.ok(Object.isFrozen(bound.target));
});
test('foreign_project_is_rejected', () => {
  const value = input(); value.target.url = 'https://framer.com/projects/different';
  assert.throws(() => validateInput(value), { message: 'target' });
});
test('invented_origin_is_rejected', () => {
  const value = input(); value.target.url = 'https://fixture.invalid/projects/offline-fixture';
  assert.throws(() => validateInput(value), { message: 'target' });
});
test('wrong_inference_route_is_rejected', () => {
  const value = input(); value.target.path = '/ai/direct-responses/';
  assert.throws(() => validateInput(value), { message: 'target' });
});
test('url_parse_error_never_contains_input', () => {
  const value = input(); value.target.url = 'private-input-not-a-url';
  assert.throws(() => validateInput(value), { message: 'target' });
});
test('unknown_configuration_and_storage_are_rejected', () => {
  assert.throws(() => validateInput({ ...input(), storage: { fake: true } }));
});
test('deadline_must_be_positive_and_bounded', () => {
  for (const budget_ms of [0, -1, 16001, Infinity, 1.5])
    assert.throws(() => validateInput({ ...input(), budget_ms }));
});
test('opaque_token_can_only_be_consumed_once', () => {
  const token = new OpaqueToken('fake-fixture-token');
  assert.equal(token.take(), 'fake-fixture-token');
  assert.throws(() => token.take(), { message: 'token-consumed' });
});
test('opaque_token_refuses_json_serialization', () => {
  assert.throws(() => JSON.stringify(new OpaqueToken('fake-fixture-token')),
    { message: 'private-token' });
});
test('original_v3_branch_is_observed', () => {
  assert.ok(exactV3(observeBranch([entry('Using Castle v3 token')])));
});
test('v2_fallback_cannot_pass_as_v3', () => {
  assert.equal(exactV3(observeBranch([entry('Using Castle v2 token')])), false);
});
test('ambiguous_branch_cannot_pass', () => {
  assert.equal(exactV3(observeBranch([
    entry('Using Castle v3 token'), entry('Using Castle v2 token'),
  ])), false);
});
test('sdk_error_blocks_successful_branch', () => {
  assert.equal(exactV3(observeBranch([
    entry('Using Castle v3 token'), entry('private error', 4),
  ])), false);
});
test('branch_reader_never_calls_entry_getters', () => {
  let called = false;
  const value = { get logger() { called = true; throw new Error('private'); } };
  assert.equal(exactV3(observeBranch([value])), false);
  assert.equal(called, false);
});
test('branch_inventory_is_bounded', () => {
  assert.throws(() => observeBranch(new Array(1001)), { message: 'logger-shape' });
});

const inferencePaths = ['/ai/v3/anthropic/messages/', '/ai/v3/chat/', '/ai/v3/messages/',
  '/ai/v3/openai/responses/', '/ai/v3/responses/'];

test('all_inference_routes_preserve_actual_project_page', () => {
  for (const path of inferencePaths) {
    for (const suffix of ['offline-fixture', 'demo--offline-fixture/',
      'demo--offline-fixture//?node=fixture#inspect']) {
      const value = input();
      value.target = { ...value.target, path, url: 'https://framer.com/projects/' + suffix };
      assert.deepEqual(validateInput(value).target, value.target);
      assert.ok(new OfflineCastleMinter(value) instanceof OfflineCastleMinter);
    }
  }
});

test('utilities_and_malformed_inference_paths_are_rejected', () => {
  for (const path of ['/ai/v3/fetch/', '/ai/v3/refund/', '/ai/v3/unsupported/',
    '/ai/v3/responses', '/ai/v3/responses/?query=1', '/auth/ai/access-token',
    'https://api.framer.com/ai/v3/responses/']) {
    const value = input(); value.target.path = path;
    assert.throws(() => validateInput(value), { message: 'target' });
  }
});

test('each_inference_route_requires_exact_project_and_origin', () => {
  for (const path of inferencePaths) {
    for (const url of ['http://framer.com/projects/offline-fixture',
      'https://example.test/projects/offline-fixture',
      'https://fixture@framer.com/projects/offline-fixture',
      'https://framer.com:8443/projects/offline-fixture',
      'https://framer.com/projects/wrong?project=offline-fixture#offline-fixture',
      'https://framer.com/projects/offline-fixture/child',
      'https://framer.com/projects/prefix-offline-fixture',
      'https://framer.com/projects/demo--offline-fixture-suffix']) {
      const value = input(); value.target = { ...value.target, path, url };
      assert.throws(() => validateInput(value), { message: 'target' });
    }
  }
});
