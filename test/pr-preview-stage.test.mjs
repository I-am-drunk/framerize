import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, readdirSync, readFileSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import nodeTest from 'node:test';
// The trusted publishing job is Linux-only and stages ZIP data with Python.
const test = process.platform === 'linux' ? nodeTest : nodeTest.skip;
const script = fileURLToPath(new URL('../.github/scripts/stage-pr-preview.py', import.meta.url));
const zipScript = `import json,sys,zipfile
with zipfile.ZipFile(sys.argv[1], 'w', zipfile.ZIP_DEFLATED) as z:
 for e in json.load(sys.stdin):
  i=zipfile.ZipInfo(e['name']); i.external_attr=e.get('mode', 0) << 16
  z.writestr(i, b'x' * e.get('bytes', 1))`;
const valid = ['preview.json', 'producer.json',
  'framerize-1.2.3-pr.23.12345.gbbbbbbbbbbbb.tgz',
  'framerize-remote-1.2.3-pr.23.12345.gbbbbbbbbbbbb.tgz'].map(name => ({ name }));

function run(t, entries) {
  const root = mkdtempSync(join(tmpdir(), 'framerize-zip-test-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const archive = join(root, 'preview.zip');
  const output = join(root, 'staged');
  execFileSync('python3', ['-c', zipScript, archive], {
    input: JSON.stringify(entries), timeout: 10_000, stdio: ['pipe', 'pipe', 'pipe'],
  });
  const result = spawnSync('python3', [script, archive, output], { timeout: 10_000, encoding: 'utf8' });
  assert.equal(result.signal, null);
  assert.equal(result.error, undefined);
  return { result, output };
}

test('ZIP staging copies only the four regular preview files', t => {
  const { result, output } = run(t, valid);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(readdirSync(output).sort(), valid.map(e => e.name).sort());
  assert.equal(readFileSync(join(output, 'preview.json'), 'utf8'), 'x');
});

const invalid = [
  ['parent traversal', { name: '../preview.json' }],
  ['absolute path', { name: '/preview.json' }],
  ['backslash alias', { name: '..\\preview.json' }],
  ['symlink', { name: 'preview.json', mode: 0o120777 }],
  ['FIFO', { name: 'preview.json', mode: 0o010600 }],
  ['directory', { name: 'preview.json', mode: 0o040755 }],
  ['oversized manifest', { name: 'preview.json', bytes: 2 * 1024 * 1024 + 1 }],
  ['duplicate path', valid[1]],
];
for (const [name, entry] of invalid) {
  test(`ZIP staging refuses ${name} before creating output`, t => {
    const { result, output } = run(t, [entry, ...valid.slice(1)]);
    assert.equal(result.status, 1);
    assert.equal(existsSync(output), false);
  });
}
