import test from 'node:test';
import assert from 'node:assert/strict';
import { createV2Window } from '../../scripts/offline_worker/v2_runtime.mjs';

class ResourceLoader {
  constructor(options) { this.options = options; }
  fetch() { assert.fail('parent resource loader must never run'); }
}
class JSDOM {
  constructor(html, options) { this.html = html; this.options = options; }
}
const page = 'https://framer.com/projects/synthetic--fixture?view=canvas#scope';
const makeWindow = (console = {}) => createV2Window({ JSDOM, ResourceLoader }, page, console);
const png = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jYzoAAAAASUVORK5CYII=',
  'base64');
const encode = bytes => 'data:image/png;base64,' + bytes.toString('base64');
const loader = () => makeWindow().options.resources;
// Header mutations test loader admission, not native PNG decoding.
function dimensions(width, height) {
  const bytes = Buffer.from(png);
  bytes.writeUInt32BE(width, 16); bytes.writeUInt32BE(height, 20);
  return encode(bytes);
}

test('v2_constructor_preserves_options_and_binds_local_png_loader', () => {
  const virtualConsole = {}, dom = makeWindow(virtualConsole);
  assert.ok(dom instanceof JSDOM);
  assert.equal(dom.html, '<!doctype html><html><head></head><body></body></html>');
  assert.deepEqual(Object.keys(dom.options).sort(),
    ['resources', 'runScripts', 'url', 'virtualConsole']);
  assert.equal(dom.options.url, page);
  assert.equal(dom.options.runScripts, 'outside-only');
  assert.equal(dom.options.virtualConsole, virtualConsole);
  assert.ok(dom.options.resources instanceof ResourceLoader);
  assert.deepEqual(dom.options.resources.options, { userAgent: 'Framerize/0.1' });
});

test('v2_png_loader_returns_bounded_bytes_without_parent_fetch', async () => {
  const pending = loader().fetch(encode(png));
  assert.equal(typeof pending?.then, 'function');
  assert.equal(typeof pending.abort, 'function');
  pending.abort();
  const bytes = await pending;
  assert.ok(Buffer.isBuffer(bytes));
  assert.deepEqual(bytes, png);
});

test('v2_png_loader_refuses_network_file_and_other_types', () => {
  const resources = loader();
  for (const value of ['https://example.invalid/image.png', 'http://127.0.0.1/image.png',
    'file:///synthetic.png', '//example.invalid/image.png',
    'blob:https://example.invalid/fixture', 'data:image/svg+xml,<svg/>',
    'data:image/jpeg;base64,' + png.toString('base64'),
    'data:text/plain;base64,' + png.toString('base64'), null, undefined, {}, 1])
    assert.equal(resources.fetch(value), null);
});

test('v2_png_loader_refuses_malformed_headers_and_noncanonical_encoding', () => {
  const resources = loader(), signature = Buffer.from(png);
  const length = Buffer.from(png), type = Buffer.from(png);
  signature[0] = 0; length.writeUInt32BE(12, 8); type.write('IDAT', 12, 'ascii');
  for (const bytes of [png.subarray(0, 32), signature, length, type])
    assert.equal(resources.fetch(encode(bytes)), null);
  for (const value of [encode(png).slice(0, -1), ' ' + encode(png), encode(png) + ' '])
    assert.equal(resources.fetch(value), null);
});

test('v2_png_loader_bounds_encoded_and_decoded_size', async () => {
  const resources = loader(), maximum = Buffer.alloc(65536);
  png.copy(maximum);
  assert.deepEqual(await resources.fetch(encode(maximum)), maximum);
  const excessive = Buffer.alloc(65537); png.copy(excessive);
  assert.equal(encode(excessive).length, 87406);
  assert.equal(resources.fetch(encode(excessive)), null);
  assert.equal(resources.fetch('data:image/png;base64,' + 'A'.repeat(87388)), null);
});

test('v2_png_loader_preserves_dimension_and_pixel_limits', async () => {
  const resources = loader();
  for (const [width, height] of [[0, 1], [1, 0], [2049, 1], [1, 2049], [513, 512]])
    assert.equal(resources.fetch(dimensions(width, height)), null);
  for (const [width, height] of [[2048, 128], [128, 2048], [512, 512]]) {
    const bytes = await resources.fetch(dimensions(width, height));
    assert.ok(Buffer.isBuffer(bytes));
    assert.equal(bytes.readUInt32BE(16), width);
    assert.equal(bytes.readUInt32BE(20), height);
  }
});

test('v2_png_quota_is_per_window_and_refusals_do_not_consume_it', async () => {
  const first = loader(), second = loader(), source = encode(png);
  for (let index = 0; index < 5; index++)
    assert.equal(first.fetch('https://example.invalid/image.png'), null);
  for (let index = 0; index < 4; index++)
    assert.deepEqual(await first.fetch(source), png);
  assert.equal(first.fetch(source), null);
  assert.deepEqual(await second.fetch(source), png);
});
