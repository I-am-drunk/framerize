import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import test from 'node:test';
import { runInNewContext } from 'node:vm';
import { gzipSync } from 'node:zlib';
import { bundleServerArchive, verifyServerSource } from '../scripts/server-native-bundle.js';
import { readPackageArchive } from '../scripts/package-archive.js';

const identity = { name: 'framerize-server', version: '0.1.0' };
test('source binding rejects substituted bytes and checks inventory before reading', () => {
  const bytes = fixture();
  const archive = readPackageArchive(bytes);
  const files = [...archive.keys()].map(file => file.slice('package/'.length));
  verifyServerSource(bytes, files, file => archive.get(`package/${file}`)!);
  assert.throws(() => verifyServerSource(bytes, files, () => Buffer.from('substitute')), /differs/);
  let reads = 0;
  assert.throws(() => verifyServerSource(bytes, [...files, 'unrelated'], () => { reads++; return Buffer.alloc(0); }), /inventory/);
  assert.equal(reads, 0);
});
test('native bundler uses only archived files and binds their compressed bytes', async () => {
  const archive = fixture();
  const result = await bundleServerArchive(archive, identity);
  const printed: string[] = [];
  runInNewContext(Buffer.from(result.code).toString(), { console: { log: (text: string) => printed.push(text) } });
  assert.deepEqual(printed, ['verified archive']);
  assert.equal(result.packageSha256, createHash('sha256').update(archive).digest('hex'));
  assert.equal(result.packageBytes, archive.length);
  assert.deepEqual(result.inputs, ['package/bin/framerize-server.js', 'package/dist/cli.js']);
});

for (const statement of ["import 'esbuild';", "import '../missing.js';",
  "import '../../../package.json';", "const file = process.argv[2]; import(file);",
  "const file = process.argv[2]; require(file);"]) {
  test(`rejects archive dependency escape: ${statement}`, async () => {
    await assert.rejects(bundleServerArchive(fixture({}, statement), identity));
  });
}
for (const changed of [{ name: 'wrong-package' }, { version: '9.0.0' }, { private: false },
  { bin: { other: './bin/other.js' } }, { dependencies: { esbuild: '*' } }]) {
  test(`rejects changed package policy ${JSON.stringify(changed)}`, async () => {
    await assert.rejects(bundleServerArchive(fixture(changed), identity));
  });
}
test('keeps Node builtin imports external', async () => {
  const result = await bundleServerArchive(fixture({}, "import 'node:fs';"), identity);
  assert.match(Buffer.from(result.code).toString(), /require\("node:fs"\)/);
});

function fixture(overrides = {}, body = "console.log('verified archive');"): Buffer {
  const manifest = { ...identity, private: true, type: 'module',
    bin: { framerize: './bin/framerize-server.js', 'framerize-server': './bin/framerize-server.js' }, ...overrides };
  const files = { 'package/package.json': JSON.stringify(manifest),
    'package/bin/framerize-server.js': "import '../dist/cli.js';", 'package/dist/cli.js': body };
  const chunks: Buffer[] = [];
  for (const [name, content] of Object.entries(files)) {
    const payload = Buffer.from(content);
    const header = Buffer.alloc(512);
    header.write(name); header.write(payload.length.toString(8).padStart(11, '0'), 124);
    header.fill(32, 148, 156); header.write('0', 156); header.write('ustar\0', 257);
    header.write(header.reduce((sum, byte) => sum + byte, 0).toString(8).padStart(6, '0') + '\0 ', 148);
    chunks.push(header, payload, Buffer.alloc((512 - payload.length % 512) % 512));
  }
  return gzipSync(Buffer.concat([...chunks, Buffer.alloc(1024)]));
}
