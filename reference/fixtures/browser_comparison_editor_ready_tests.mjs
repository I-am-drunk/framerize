import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

const source = readFileSync(new URL('../../scripts/contextual_authorization.py', import.meta.url), 'utf8');
const marker = 'COMPARISON_READY = """';
const start = source.indexOf(marker);
assert.ok(start >= 0);
assert.equal(source.indexOf(marker, start + marker.length), -1);
const end = source.indexOf('"""', start + marker.length);
const literal = source.slice(start + marker.length, end);
assert.equal(literal.includes('\\'), false);
const project = 'fixture-project';
const page = 'https://framer.com/projects/Fixture--' + project + '?node=fixture-node';
const state = () => ({ timestamp: 123456, value: {
  mainView: 0, branchId: 'main', activeScopeId: 'fixture-node',
  collectionItemId: null, overlay: null,
} });
const checkNames = ['project_match', 'history_object', 'timestamp_valid', 'transient_absent',
  'canvas_view', 'branch_valid', 'scope_valid', 'device_present'];

function ready(value = state(), href = page, device = 'fixture-device', diagnose = false) {
  const poll = new Function('location', 'history', 'localStorage', 'URL',
    'return (' + literal + ');')({ href }, { state: value }, {
      getItem(key) { assert.equal(key, '__cuid'); return device; },
    }, URL);
  return poll({ project, diagnose });
}

test('diagnostic mode returns only the eight readiness booleans', () => {
  const checks = ready(state(), page, 'private-fixture-device', true);
  assert.deepEqual(checks, Object.fromEntries(checkNames.map(name => [name, true])));
  const missing = ready(null, page, 'private-fixture-device', true);
  assert.deepEqual(missing, {
    project_match: true, history_object: false, timestamp_valid: false, transient_absent: false,
    canvas_view: false, branch_valid: false, scope_valid: false, device_present: true,
  });
  assert.equal(JSON.stringify([checks, missing]).includes('private-fixture'), false);
});

test('probe facts and predicate agree on each refused readiness condition', () => {
  const cases = [
    ['project_match', state(), 'https://framer.com/projects/other', 'device'],
    ['history_object', null, page, 'device'],
    ['timestamp_valid', { ...state(), timestamp: 0 }, page, 'device'],
    ['transient_absent', { ...state(), isTransient: false }, page, 'device'],
    ['canvas_view', { ...state(), value: { ...state().value, mainView: '0' } }, page, 'device'],
    ['branch_valid', { ...state(), value: { ...state().value, branchId: '' } }, page, 'device'],
    ['scope_valid', { ...state(), value: { ...state().value, activeScopeId: '' } }, page, 'device'],
    ['device_present', state(), page, ''],
  ];
  for (const [failed, value, href, device] of cases) {
    const checks = ready(value, href, device, true);
    assert.equal(checks[failed], false, failed);
    assert.equal(Object.values(checks).every(value => typeof value === 'boolean'), true);
    assert.equal(ready(value, href, device), Object.values(checks).every(value => value === true));
  }
});

test('failed state access throws instead of fabricating diagnostic facts', () => {
  const value = state();
  Object.defineProperty(value, 'timestamp', { get() { throw new Error('private-fixture'); } });
  assert.throws(() => ready(value, page, 'device', true));
  assert.throws(() => ready(value));
});

test('completed canvas state permits the expected full project URL', () => {
  assert.equal(ready(), true);
  assert.equal(ready(state(), 'https://framer.com/projects/' + project), true);
  const alternate = state(); alternate.value.branchId = 'other-branch';
  assert.equal(ready(alternate), true);
});

test('device readiness and a canonical slug alone do not release the wait', () => {
  const device = 'fixture-device';
  assert.equal(typeof device === 'string' && device.length > 0, true);
  assert.equal(ready(null, page, device), false);
  assert.equal(ready({ timestamp: 123456, value: undefined }), false);
});

test('initial route publication precedes readiness in the simulated startup', () => {
  assert.equal(ready(null, 'https://framer.com/projects/' + project), false);
  assert.equal(ready(null, page), false);
  assert.equal(ready(state(), page), true);
});

test('foreign origin or project cannot satisfy readiness', () => {
  for (const href of ['https://example.com/projects/' + project,
    'https://framer.com/projects/another-project',
    'https://framer.com/projects/nested/' + project,
    'https://user:password@framer.com/projects/' + project,
    'https://framer.com:444/projects/' + project, 'https://framer.com/login']) {
    assert.equal(ready(state(), href), false);
  }
});

test('transient and malformed history envelopes remain pending', () => {
  for (const value of [null, [], true, {}, { timestamp: '123', value: state().value },
    ...[NaN, Infinity, 0, -1].map(timestamp => ({ ...state(), timestamp })),
    ...[true, false, null, 0].map(isTransient => ({ ...state(), isTransient }))]) {
    assert.equal(ready(value), false);
  }
});

test('canvas view and bounded nonempty branch and scope are required', () => {
  for (const mainView of [undefined, null, true, '0', 1, 2, 3, 4, 5, 6]) {
    const value = state(); value.value.mainView = mainView;
    assert.equal(ready(value), false);
  }
  for (const key of ['branchId', 'activeScopeId']) {
    for (const bad of [undefined, null, true, 1, '', 'x'.repeat(257)]) {
      const value = state(); value.value[key] = bad;
      assert.equal(ready(value), false);
    }
  }
});

test('history readiness still requires existing device storage', () => {
  for (const device of [null, '']) assert.equal(ready(state(), page, device), false);
});

test('the predicate reads frozen state without rewriting it', () => {
  const value = state();
  Object.freeze(value.value); Object.freeze(value);
  const before = JSON.stringify(value);
  assert.equal(ready(value), true);
  assert.equal(Object.values(ready(value, page, 'fixture-device', true)).every(Boolean), true);
  assert.equal(JSON.stringify(value), before);
});
