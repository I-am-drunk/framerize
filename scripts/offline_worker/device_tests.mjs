import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { DOM } from './pins.mjs';
import { validateInput } from './protocol.mjs';
import { SavedDevice } from './device.mjs';
import { OfflineCastleMinter } from './offline_mint.mjs';
const input = () => ({ version: 2, budget_ms: 16000, device: { id: 'fixture-device-a' },
  target: { project_id: 'fixture', path: '/ai/v3/responses/',
    url: 'https://framer.com/projects/fixture' } });
const dom = () => {
  const { JSDOM } = createRequire(import.meta.url)(DOM + '/node_modules/jsdom/lib/api.js');
  return new JSDOM('', { url: input().target.url, runScripts: 'outside-only' });
};

test('saved_device_constructor_is_inert', () => {
  assert.ok(new OfflineCastleMinter(input()) instanceof OfflineCastleMinter);
});
test('saved_device_requires_explicit_bounded_safe_value', () => {
  for (const device of [undefined, {}, { id: '' }, { id: 'a'.repeat(257) },
    { id: 'private;value' }, { id: 'private\nvalue' }, { id: 'a', unknown: true }]) {
    assert.throws(() => validateInput({ ...input(), device }), { message: 'device' });
  }
});
test('stateless_protocol_does_not_accept_device_claim', () => {
  assert.throws(() => validateInput({ ...input(), version: 1 }), { message: 'input' });
});

test('saved_device_is_copied_and_not_serializable', () => {
  const value = input(), bound = validateInput(value);
  value.device.id = 'changed';
  assert.ok(Object.isFrozen(bound.device));
  assert.throws(() => JSON.stringify(bound), { message: 'private-device' });
  const instance = dom();
  try {
    bound.device.seed(instance.window)();
    assert.equal(instance.window.localStorage.getItem('__cuid'), 'fixture-device-a');
  } finally { instance.window.close(); }
});

test('saved_device_seeds_real_local_storage_and_cookie', () => {
  const instance = dom();
  try {
    const verify = validateInput(input()).device.seed(instance.window);
    assert.equal(instance.window.localStorage.getItem('__cuid'), 'fixture-device-a');
    assert.equal(instance.window.document.cookie, '__cuid=fixture-device-a');
    instance.window.localStorage.setItem('__cuid', 'fixture-device-a');
    instance.window.document.cookie = '__cuid=fixture-device-a; Path=/; Secure';
    verify();
  } finally { instance.window.close(); }
});

test('saved_devices_remain_isolated_across_windows', () => {
  const a = dom(), b = dom();
  try {
    const verifyA = new SavedDevice({ id: 'fixture-device-a' }).seed(a.window);
    const verifyB = new SavedDevice({ id: 'fixture-device-b' }).seed(b.window);
    verifyA(); verifyB();
    assert.equal(a.window.document.cookie, '__cuid=fixture-device-a');
    assert.equal(b.window.document.cookie, '__cuid=fixture-device-b');
  } finally { a.window.close(); b.window.close(); }
});

test('saved_device_reuses_explicit_value_across_fresh_windows', () => {
  for (let i = 0; i < 2; i++) {
    const instance = dom();
    try {
      new SavedDevice({ id: 'fixture-device-a' }).seed(instance.window)();
      assert.equal(instance.window.localStorage.getItem('__cuid'), 'fixture-device-a');
    } finally { instance.window.close(); }
  }
});

test('changed_saved_storage_fails_without_creating_a_replacement', () => {
  for (const field of ['local', 'cookie']) {
    const instance = dom();
    try {
      const verify = new SavedDevice({ id: 'fixture-device-a' }).seed(instance.window);
      if (field === 'local') instance.window.localStorage.setItem('__cuid', 'changed');
      else instance.window.document.cookie = '__cuid=changed; Path=/; Secure';
      assert.throws(verify, { message: 'device-storage-changed' });
      if (field === 'local') assert.equal(instance.window.localStorage.getItem('__cuid'), 'changed');
      else assert.equal(instance.window.document.cookie, '__cuid=changed');
    } finally { instance.window.close(); }
  }
});

test('saved_device_refuses_existing_storage_and_reseeding', () => {
  const instance = dom();
  try {
    instance.window.localStorage.setItem('existing', 'value');
    const device = new SavedDevice({ id: 'fixture-device-a' });
    assert.throws(() => device.seed(instance.window), { message: 'device-storage-not-empty' });
    instance.window.localStorage.clear();
    assert.throws(() => device.seed(instance.window), { message: 'device-already-seeded' });
    assert.equal(instance.window.document.cookie, '');
  } finally { instance.window.close(); }
});
