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
    'scripts/server-native-bundle.ts', 'scripts/native-account-smoke.ts',
    'scripts/package-framer-next.ts', 'test/framer-next.test.ts',
    'test/server-native-bundle.test.ts', 'test/server-native-account.test.ts']);
  assert.doesNotMatch(root.scripts['build:server'], /framerize-remote/);
  assert.match(root.scripts['build:server'], /^npm --prefix framerize-castle run build &&/);
  assert.match(read('scripts/build-server.ts'), /stageCastleRuntime\(root\)/);
  assert.equal((workflow.match(/npm run build:server --prefix \.\./g) ?? []).length, 3);
  assert.doesNotMatch(workflow, /npm ci\s*\n|npm run build --prefix/);
  assert.match(workflow, /NODE_OPTIONS: ''/);
  assert.equal((workflow.match(/persist-credentials: false/g) ?? []).length, 4);
  assert.match(workflow, /npm run test:native-tools --prefix \.\./);
});

test('server workflows cover changed inputs and keep all pinned platforms', () => {
  for (const file of ['.gitattributes', 'scripts/server-native-bundle.ts', 'scripts/package-archive.ts',
    'framerize-castle/**', 'scripts/server-castle-runtime.ts', 'scripts/native-account-smoke.ts',
    'scripts/core-package-check.ts', 'scripts/fixtures/**', 'scripts/*framer-next*',
    'test/framer-next.test.ts', 'test/server-*']) {
    assert.equal(workflow.split(`'${file}'`).length - 1, 2, file);
  }
  assert.match(workflow, /node: \['22\.12\.0', '22\.23\.4', '24\.13\.0'\]/);
  for (const runner of ['ubuntu-22.04', 'macos-15', 'windows-2022']) assert.ok(workflow.includes(runner));
  assert.equal((workflow.match(/overwrite: true/g) ?? []).length, 4);
  assert.equal((workflow.match(/if-no-files-found: error/g) ?? []).length, 4);
  verifyNativeOrder(workflow);
});

function verifyNativeOrder(source) {
  const native = source.replace(/\r\n/g, '\n').split('\n  native:\n')[1];
  assert.ok(native, 'Missing native job');
  const checked = native.indexOf('npm run pack:check -- --downloaded');
  const built = native.indexOf('npm run binary\n');
  assert.ok(checked >= 0, 'Missing downloaded-package check');
  assert.ok(built > checked, 'Native build must follow the downloaded-package check');
  const smoke = native.indexOf('npm run binary:check');
  const metadata = native.indexOf('node scripts/release-metadata.js');
  assert.ok(smoke > built && metadata > smoke, 'Native smoke must pass before release metadata');
  assert.doesNotMatch(native, /continue-on-error|\|\| true/);
}

for (const newline of ['\n', '\r\n']) {
  test(`native build ordering accepts ${newline === '\n' ? 'LF' : 'CRLF'} only with its check`, () => {
    const source = workflow.slice(workflow.indexOf('\n  native:')).replace(/\r?\n/g, newline);
    verifyNativeOrder(source);
    assert.throws(() => verifyNativeOrder(source.replace('npm run pack:check -- --downloaded', 'omitted')));
    assert.throws(() => verifyNativeOrder(source.replace(`npm run binary${newline}`, `omitted${newline}`)));
    assert.throws(() => verifyNativeOrder(source.replace('npm run binary:check', 'omitted')));
    assert.throws(() => verifyNativeOrder(source.replace('node scripts/release-metadata.js', 'omitted')));
    assert.throws(() => verifyNativeOrder(source.replace('npm run pack:check -- --downloaded',
      `npm run binary${newline}npm run pack:check -- --downloaded`)));
  });
}

test('native launcher consumes the verified archive rather than source files', () => {
  const builder = read('framerize-server/scripts/build-binary.js');
  assert.match(builder, /bundleServerArchive\(archiveBytes, identity\)/);
  assert.ok(builder.indexOf('verifyServerSource(archiveBytes') < builder.indexOf('await bundleServerArchive'));
  assert.match(builder, /packageSha256: bundle\.packageSha256/);
  assert.doesNotMatch(builder, /entryPoints|from 'esbuild'/);
});

test('native smoke awaits account verification on the extracted executable', () => {
  const source = read('framerize-server/scripts/binary-smoke.js');
  const validate = input => {
    assert.match(input, /await checkNativeAccount\(executable\)/);
    const bound = input.indexOf('digest(path.join(directory, entry))');
    assert.ok(bound >= 0 && input.indexOf('await checkNativeAccount(executable)') > bound);
  };
  validate(source);
  assert.throws(() => validate(source.replace('await checkNativeAccount', 'checkNativeAccount')));
  assert.throws(() => validate(source.replace('checkNativeAccount(executable)', 'checkNativeAccount(process.execPath)')));
  assert.throws(() => validate(source.replace('digest(path.join(directory, entry))', 'unverified')));
  assert.match(read('package.json'), /\.tools\/test\/server-native-account\.test\.js/);
  const core = read('.github/workflows/core-ci.yml');
  assert.equal(core.split("'scripts/native-account-smoke.ts'").length - 1, 2);
});
