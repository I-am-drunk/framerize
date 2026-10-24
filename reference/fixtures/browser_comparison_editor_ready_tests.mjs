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

function ready(value = state(), href = page, device = 'fixture-device') {
  const poll = new Function('location', 'history', 'localStorage', 'URL',
    'return (' + literal + ');')({ href }, { state: value }, {
      getItem(key) { assert.equal(key, '__cuid'); return device; },
    }, URL);
  return poll({ project });
}

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
  assert.equal(JSON.stringify(value), before);
});
