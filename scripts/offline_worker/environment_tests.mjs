import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { DOM } from './pins.mjs';
import { createOfflineWindow } from './offline_mint.mjs';

const jsdom = createRequire(import.meta.url)(DOM + '/node_modules/jsdom/lib/api.js');
const page = 'https://framer.com/projects/fixture-project';

test('offline_window_keeps_native_identity_and_page_context', () => {
  const instance = createOfflineWindow(jsdom, page, new jsdom.VirtualConsole());
  try {
    assert.equal(instance.window.navigator.userAgent, 'Framerize/0.1');
    assert.equal(instance.window.location.href, page);
    assert.equal(instance.window.document.hidden, false);
    assert.equal(instance.window.document.visibilityState, 'visible');
    assert.equal(instance.window.localStorage.length, 0);
    assert.equal(instance.window.document.cookie, '');
    assert.equal(instance.window.Worker, undefined);
  } finally { instance.window.close(); }
});

test('offline_window_keeps_all_resource_fetches_disabled', () => {
  let loader;
  class TrapLoader extends jsdom.ResourceLoader {
    constructor(options) { super(options); loader = this; }
    fetch() { assert.fail('JSDOM base resource loader must not run'); }
  }
  const instance = createOfflineWindow({ ...jsdom, ResourceLoader: TrapLoader },
    page, new jsdom.VirtualConsole());
  try {
    for (const url of ['https://example.invalid/fixture',
      'file:///not-a-real-fixture', 'data:text/plain,fixture']) {
      assert.equal(loader.fetch(url), null);
    }
  } finally { instance.window.close(); }
});
