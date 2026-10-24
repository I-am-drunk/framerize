import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { DOM } from '../../scripts/offline_worker/pins.mjs';
import { createV2Window } from '../../scripts/offline_worker/v2_runtime.mjs';

// Run with the explicitly qualified native-canvas DOM pins, never a live page.
const jsdom = createRequire(import.meta.url)(DOM + '/node_modules/jsdom/lib/api.js');
function load(image, source) {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('inline PNG did not load')), 2000);
    image.onload = () => { clearTimeout(timeout); resolve(); };
    image.onerror = () => { clearTimeout(timeout); reject(new Error('inline PNG failed')); };
    image.src = source;
  });
}

test('v2_png_window_preserves_the_default_navigator_user_agent', () => {
  const url = 'https://framer.com/projects/fixture';
  const baseline = new jsdom.JSDOM('', { url, runScripts: 'outside-only' });
  const actual = createV2Window(jsdom, url, new jsdom.VirtualConsole());
  try {
    assert.equal(actual.window.navigator.userAgent, baseline.window.navigator.userAgent);
  } finally { actual.window.close(); baseline.window.close(); }
});

test('v2_window_decodes_native_canvas_png_without_changing_page_capabilities', async () => {
  const url = 'https://framer.com/projects/fixture?retained=1';
  const dom = createV2Window(jsdom, url, new jsdom.VirtualConsole());
  try {
    assert.equal(dom.window.location.href, url);
    assert.equal(dom.window.document.hidden, true);
    assert.equal(dom.window.requestAnimationFrame, undefined);
    assert.equal(dom.window.Worker, undefined);
    assert.equal(dom.window.SharedWorker, undefined);
    const canvas = dom.window.document.createElement('canvas');
    canvas.width = canvas.height = 2;
    const context = canvas.getContext('2d');
    context.fillStyle = '#ff0000'; context.fillRect(0, 0, 1, 1);
    assert.equal(canvas.getContext('webgl'), null);
    const image = new dom.window.Image();
    await load(image, canvas.toDataURL());
    context.clearRect(0, 0, 2, 2); context.drawImage(image, 0, 0);
    assert.deepEqual([...context.getImageData(0, 0, 1, 1).data], [255, 0, 0, 255]);
    assert.equal(canvas.getContext('webgl'), null);
  } finally { dom.window.close(); }
});
