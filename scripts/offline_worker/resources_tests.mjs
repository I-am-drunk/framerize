import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { DOM } from './pins.mjs';
import { pngResources } from './resources.mjs';
import { createOfflineWindow } from './offline_mint.mjs';
const jsdom = createRequire(import.meta.url)(DOM + '/node_modules/jsdom/lib/api.js');
const Resources = pngResources(class {
  fetch() { assert.fail('base loader must never run'); }
});
const makeWindow = () => createOfflineWindow(jsdom, 'https://framer.com/projects/fixture',
  new jsdom.VirtualConsole());
const png = () => {
  const window = makeWindow();
  try { return window.window.document.createElement('canvas').toDataURL(); }
  finally { window.window.close(); }
};

test('native_canvas_png_roundtrip_keeps_unsupported_contexts_absent', async () => {
  const dom = makeWindow();
  try {
    const canvas = dom.window.document.createElement('canvas');
    canvas.width = canvas.height = 2;
    assert.equal(canvas.getContext('webgl'), null);
    const context = canvas.getContext('2d');
    context.fillStyle = '#ff0000'; context.fillRect(0, 0, 1, 1);
    assert.deepEqual([...context.getImageData(0, 0, 1, 1).data], [255, 0, 0, 255]);
    assert.equal(canvas.getContext('webgl'), null);
    const image = new dom.window.Image();
    await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('image timeout')), 2000);
      image.onload = () => { clearTimeout(timeout); resolve(); };
      image.onerror = () => { clearTimeout(timeout); reject(new Error('image failure')); };
      image.src = canvas.toDataURL();
    });
    context.clearRect(0, 0, 2, 2); context.drawImage(image, 0, 0);
    assert.deepEqual([...context.getImageData(0, 0, 1, 1).data], [255, 0, 0, 255]);
    assert.equal(dom.window.Worker, undefined);
    assert.equal(dom.window.SharedWorker, undefined);
  } finally { dom.window.close(); }
});

test('png_loader_never_opens_external_or_unsupported_resources', () => {
  const loader = new Resources();
  for (const input of ['https://example.invalid/image.png', 'file:///etc/passwd',
    'data:image/svg+xml,<svg/>', 'data:text/plain,hello', 'data:image/png;base64,AAAA',
    'data:image/png;base64,' + 'A'.repeat(90000), null, {}])
    assert.equal(loader.fetch(input), null);
});

test('png_quota_is_per_window_and_does_not_reject_another_window', async () => {
  const source = png(), first = new Resources(), second = new Resources();
  for (let i = 0; i < 4; i++) assert.ok(Buffer.isBuffer(await first.fetch(source)));
  assert.equal(first.fetch(source), null);
  assert.ok(Buffer.isBuffer(await second.fetch(source)));
});

test('png_headers_and_decoded_bytes_have_independent_limits', () => {
  const bytes = Buffer.from(png().split(',')[1], 'base64');
  const loader = new Resources();
  const encode = value => 'data:image/png;base64,' + value.toString('base64');
  for (const [width, height] of [[0, 1], [2049, 1], [1024, 1024]]) {
    const altered = Buffer.from(bytes);
    altered.writeUInt32BE(width, 16); altered.writeUInt32BE(height, 20);
    assert.equal(loader.fetch(encode(altered)), null);
  }
  const excessive = Buffer.alloc(65537); bytes.copy(excessive);
  assert.equal(loader.fetch(encode(excessive)), null);
  const malformed = Buffer.from(bytes); malformed[0] = 0;
  assert.equal(loader.fetch(encode(malformed)), null);
  const padded = Buffer.concat([bytes, Buffer.alloc((4 - bytes.length % 3) % 3)]);
  assert.equal(loader.fetch(encode(padded).replace(/=+$/, '')), null);
});
