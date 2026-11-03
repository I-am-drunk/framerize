import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

const read = file => readFileSync(new URL('../' + file, import.meta.url), 'utf8');
const workflow = read('.github/workflows/server-ci.yml');
test('server builds use only the typed server target and safe installs', () => {
  const root = JSON.parse(read('package.json'));
  assert.match(root.scripts['build:server'], /tsc -p framerize-server\/tsconfig.json/);
  assert.match(root.scripts['build:server'], /tsc -p tsconfig\.server-tools\.json/);
  const tools = JSON.parse(read('tsconfig.server-tools.json'));
  assert.deepEqual(tools.include, ['scripts/build-server.ts',
    'scripts/server-native-bundle.ts', 'test/server-native-bundle.test.ts']);
  assert.doesNotMatch(root.scripts['build:server'], /framerize-(?:remote|castle)/);
  assert.equal((workflow.match(/npm run build:server --prefix \.\./g) ?? []).length, 3);
  assert.doesNotMatch(workflow, /npm ci\s*\n|npm run build --prefix/);
  assert.match(workflow, /NODE_OPTIONS: ''/);
  assert.equal((workflow.match(/persist-credentials: false/g) ?? []).length, 4);
  assert.match(workflow, /npm run test:native-tools --prefix \.\./);
});

test('server workflows cover changed inputs and keep all pinned platforms', () => {
  for (const file of ['scripts/server-native-bundle.ts', 'scripts/package-archive.ts', 'test/server-*']) {
    assert.equal(workflow.split(`'${file}'`).length - 1, 2, file);
  }
  assert.match(workflow, /node: \['22\.12\.0', '22\.23\.4', '24\.13\.0'\]/);
  for (const runner of ['ubuntu-22.04', 'macos-15', 'windows-2022']) assert.ok(workflow.includes(runner));
  assert.equal((workflow.match(/overwrite: true/g) ?? []).length, 3);
  assert.equal((workflow.match(/if-no-files-found: error/g) ?? []).length, 3);
  assert.ok(workflow.indexOf('npm run pack:check -- --downloaded', workflow.indexOf('  native:'))
    < workflow.indexOf('npm run binary\n'));
});

test('native launcher consumes the verified archive rather than source files', () => {
  const builder = read('framerize-server/scripts/build-binary.js');
  assert.match(builder, /bundleServerArchive\(archiveBytes, identity\)/);
  assert.ok(builder.indexOf('verifyServerSource(archiveBytes') < builder.indexOf('await bundleServerArchive'));
  assert.match(builder, /packageSha256: bundle\.packageSha256/);
  assert.doesNotMatch(builder, /entryPoints|from 'esbuild'/);
});
