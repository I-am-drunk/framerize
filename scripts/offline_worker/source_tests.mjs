import test from 'node:test';
import assert from 'node:assert/strict';
import { validateInput, OpaqueToken } from './protocol.mjs';
import { observeBranch, exactV3 } from './branch.mjs';
import { OfflineCastleMinter } from './offline_mint.mjs';

const input = () => ({ version: 1, budget_ms: 16000, target: {
  project_id: 'offline-fixture', path: '/ai/v3/responses/',
  url: 'https://framer.com/projects/offline-fixture',
} });
const entry = (message, level = 1) => ({ logger: { id: 'castle' }, level, parts: [message] });

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
