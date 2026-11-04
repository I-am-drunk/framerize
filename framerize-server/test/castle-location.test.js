import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { win32 } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { runInNewContext } from 'node:vm';
import test from 'node:test';

test('virtual assets keep the same Windows root across current-drive changes', () => {
  const generator = readFileSync(new URL('../../scripts/vendor-castle-library.mjs', import.meta.url), 'utf8');
  const match = generator.match(/modules\.set\('location', `([^`]+)`\);/);
  assert.ok(match, 'Expected the bundled virtual location module');
  const code = match[1].replace(/^import .*;\n/gm, '').replaceAll('export const ', 'const ');
  const roots = ['C', 'D'].map(drive => runInNewContext(`${code}\nroot;`, {
    process: { platform: 'win32' }, URL, sep: '\\',
    resolve: path => win32.resolve(`${drive}:\\synthetic`, path),
    fileURLToPath: url => fileURLToPath(url, { windows: true }),
    pathToFileURL: path => pathToFileURL(path, { windows: true }),
  }));
  assert.equal(roots[0], roots[1]);
  assert.ok(win32.isAbsolute(roots[0]));
  assert.equal(win32.resolve(roots[0], 'castle/inputs/interaction-trace.json'),
    'C:\\__castle_library__\\castle\\inputs\\interaction-trace.json');
});
