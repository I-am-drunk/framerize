import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import { createHash } from 'node:crypto';
import { loadGraphics } from './graphics_loader.mjs';
const hash = data => createHash('sha256').update(data).digest('hex');
function fixture(name) {
  const root = '/out/' + name;
  fs.mkdirSync(root + '/src/javascript', { recursive: true });
  fs.writeFileSync(root + '/index.js', 'module.exports = () => 42;');
  fs.writeFileSync(root + '/src/javascript/native-gl.js',
    'exports.NativeWebGL = { cleanup: () => true, activeContexts: () => 0 };');
  const entries = Object.fromEntries(['index.js', 'src/javascript/native-gl.js']
    .map(file => [root + '/' + file, hash(fs.readFileSync(root + '/' + file))]));
  const manifest = JSON.stringify(entries);
  fs.writeFileSync(root + '/files.json', manifest);
  return { root, manifestSha256: hash(manifest) };
}
test('rejects a changed manifest before importing graphics', () => {
  const spec = fixture('manifest-case');
  fs.appendFileSync(spec.root + '/files.json', ' ');
  assert.throws(() => loadGraphics(spec), /graphics manifest/);
});
test('rejects changed module bytes before importing them', () => {
  const spec = fixture('module-case');
  fs.writeFileSync(spec.root + '/index.js', 'throw new Error("must not execute");');
  assert.throws(() => loadGraphics(spec), /graphics input/);
});
test('verifies modules again after use', () => {
  const spec = fixture('after-use-case');
  const loaded = loadGraphics(spec);
  assert.equal(loaded.create(), 42);
  assert.equal(loaded.cleanup(), true);
  assert.equal(loaded.activeContexts(), 0);
  loaded.verify();
  fs.appendFileSync(spec.root + '/index.js', '\n');
  assert.throws(() => loaded.verify(), /graphics input/);
});
