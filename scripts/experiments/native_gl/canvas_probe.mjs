import assert from 'node:assert/strict';
import fs from 'node:fs';
import { createRequire } from 'node:module';
import { installWebGL } from './canvas_webgl.mjs';
const require = createRequire(import.meta.url);
const { JSDOM } = require('/dom/node_modules/jsdom');
const create = require('/work/implementation');
const { NativeWebGL } = require('/work/implementation/src/javascript/native-gl');
const dom = new JSDOM('<!doctype html>');
const original = dom.window.HTMLCanvasElement.prototype.getContext;
const graphics = installWebGL(dom.window, create, NativeWebGL.cleanup);
const canvas = () => dom.window.document.createElement('canvas');
const checks = [];
try {
  const two = canvas();
  assert.ok(two.getContext('2d'));
  assert.equal(two.getContext('webgl'), null);
  checks.push('2d_excludes_webgl');
  const first = canvas(), gl = first.getContext('webgl');
  assert.ok(gl); assert.equal(gl.canvas, first);
  assert.equal(first.getContext('experimental-webgl'), gl);
  assert.equal(first.getContext('2d'), null);
  checks.push('stable_context_and_type');
  first.width = 32;
  assert.equal(first.getContext('webgl'), gl);
  assert.equal(gl.drawingBufferWidth, 32);
  first.width = 2048;
  assert.throws(() => first.getContext('webgl'), /resource limit/);
  checks.push('bounded_resize');
  first.width = 32;
  assert.ok(canvas().getContext('webgl'));
  assert.throws(() => canvas().getContext('webgl'), /resource limit/);
  checks.push('context_limit');
  const result = graphics.close();
  assert.deepEqual(result, { created: 2, destroyed: 2, refused: 2, errors: 0 });
  assert.equal(graphics.close(), result);
  assert.equal(dom.window.HTMLCanvasElement.prototype.getContext, original);
  checks.push('confirmed_cleanup_and_restore');
  const next = installWebGL(dom.window, create, NativeWebGL.cleanup);
  assert.ok(canvas().getContext('webgl'));
  assert.deepEqual(next.close(), { created: 1, destroyed: 1, refused: 0, errors: 0 });
  checks.push('new_owner_after_cleanup');
} finally {
  graphics.close(); dom.window.close();
}
fs.writeFileSync('/out/probe.json', JSON.stringify({ passed: true, checks,
  sdk_executed: false }, null, 2) + '\n', { flag: 'wx' });
