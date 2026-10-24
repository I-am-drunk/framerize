// Inert protocol validation only. No SDK, browser or requests.
import test from 'node:test';
import assert from 'node:assert/strict';
import { validateInput } from '../../scripts/offline_worker/protocol.mjs';

const PROJECT = 'AB12cd34EF56gh78IJ90';
const PATHS = ['/ai/v3/responses/', '/ai/v3/messages/', '/ai/v3/chat/',
  '/ai/v3/openai/responses/', '/ai/v3/anthropic/messages/'];

test('access_links_preserve_exact_url_for_all_inference_routes', () => {
  for (const path of PATHS) {
    for (const suffix of [`${PROJECT}-Access12`, `demo-title--${PROJECT}-Access12/`,
      `demo--${PROJECT}-Access12//?node=fixture#inspect`]) {
      const value = input(suffix, path);
      const target = validateInput(value).target;
      assert.deepEqual(target, value.target);
      assert.ok(Object.isFrozen(target));
    }
  }
});

test('saved_device_input_keeps_the_original_access_link', () => {
  const value = { ...input(`${PROJECT}-Access12`), version: 2, device: { id: 'fixture' } };
  assert.deepEqual(validateInput(value).target, value.target);
});

test('access_links_reject_another_project_despite_slug_query_or_fragment', () => {
  const other = 'XY12cd34EF56gh78IJ90';
  for (const suffix of [`${other}-Access12?project=${PROJECT}#${PROJECT}`,
    `${PROJECT}--${other}-Access12`, `${PROJECT.toLowerCase()}-Access12`]) {
    assert.throws(() => validateInput(input(suffix)), { message: 'target' });
  }
});

test('malformed_access_links_remain_rejected', () => {
  for (const suffix of [`${PROJECT}-`, `${PROJECT}-Access-with-dashes`,
    `${PROJECT}-A%2FB`, `demo_--${PROJECT}-Access12`,
    `-demo--${PROJECT}-Access12`, `${PROJECT.slice(1)}-Access12`,
    `${PROJECT}x-Access12`, `${PROJECT}-Access12/child`]) {
    assert.throws(() => validateInput(input(suffix)), { message: 'target' });
  }
});

test('access_link_does_not_relax_origin_or_route_checks', () => {
  for (const origin of ['http://framer.com', 'https://fixture@framer.com',
    'https://framer.com:8443', 'https://other.framer.com']) {
    const value = input(`${PROJECT}-Access12`);
    value.target.url = `${origin}/projects/${PROJECT}-Access12`;
    assert.throws(() => validateInput(value), { message: 'target' });
  }
  assert.throws(() => validateInput(input(`${PROJECT}-Access12`, '/ai/v3/refund/')),
    { message: 'target' });
});

test('legacy_project_forms_do_not_gain_arbitrary_suffixes', () => {
  for (const suffix of ['offline-fixture', 'demo--offline-fixture']) {
    const value = input(suffix);
    value.target.project_id = 'offline-fixture';
    assert.deepEqual(validateInput(value).target, value.target);
    value.target.url += '-Access12';
    assert.throws(() => validateInput(value), { message: 'target' });
  }
});

function input(suffix, path = PATHS[0]) {
  return { version: 1, budget_ms: 16000, target: { project_id: PROJECT, path,
    url: 'https://framer.com/projects/' + suffix } };
}
