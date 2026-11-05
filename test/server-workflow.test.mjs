import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join, posix, resolve } from 'node:path';
import { gunzipSync } from 'node:zlib';
import { createRequire } from 'node:module';
import test from 'node:test';
import { runMerge, seedRelease } from '../framerize-server/test/helpers/release-fixtures.js';

const read = file => readFileSync(new URL('../' + file, import.meta.url), 'utf8');
const workflow = read('.github/workflows/server-ci.yml');
test('native uploads retain every file needed to assemble release manifests', context => {
  const upload = workflow.replace(/\r\n/g, '\n')
    .split('name: framerize-server-native-${{ matrix.target }}\n')[1];
  assert.ok(upload, 'Missing native artifact upload');
  const paths = upload.match(/^          path: \|\n((?:            .+\n)+)/m)?.[1]
    .trim().split('\n').map(line => line.trim());
  assert.ok(paths?.length, 'Missing native upload paths');
  const checkout = seedRelease(context);
  for (const target of Object.values(checkout.targets)) {
    for (const name of readdirSync(target.dist)) {
      const source = `framerize-server/dist/${name}`;
      if (!paths.some(pattern => posix.matchesGlob(source, pattern))) {
        rmSync(join(target.dist, name));
      }
    }
  }
  const result = runMerge(checkout);
  assert.equal(result.error, undefined, result.error?.message);
  assert.equal(result.status, 0, result.stderr);
});
test('recovery archive supports drive paths and spaces', context => {
  const root = mkdtempSync(join(tmpdir(), 'framer-recovery-'));
  context.after(() => rmSync(root, { recursive: true, force: true }));
  const runner = process.platform === 'win32' ? join(root, 'runner with spaces') : 'D:/runner with spaces';
  const archive = `${runner}/framer-next-windows-x64.tar.gz`;
  const directory = resolve(root, runner, 'framer-next');
  mkdirSync(directory, { recursive: true });
  writeFileSync(join(directory, 'fixture.txt'), 'synthetic-native-bytes');
  assert.match(workflow,
    /tar --format=ustar -czf - -C "\$RUNNER_TEMP" framer-next > "\$RECOVERY_ARCHIVE"/);
  const command = workflow.match(/^\s*(tar --format=ustar[^\n]+)/m)?.[1];
  assert.ok(command, 'Missing recovery archive command');
  assert.match(command, /-czf - -C "\$RUNNER_TEMP" framer-next > "\$RECOVERY_ARCHIVE"/);
  const env = { PATH: process.env.PATH, SystemRoot: process.env.SystemRoot,
    HOME: root, USERPROFILE: root, RUNNER_TEMP: runner, RECOVERY_ARCHIVE: archive };
  const shell = ['--noprofile', '--norc', '-euo', 'pipefail', '-c'];
  const version = spawnSync('bash', [...shell, 'tar --version'], { env, encoding: 'utf8' });
  assert.equal(version.status, 0, version.stderr);
  if (version.stdout.includes('GNU tar')) env.TAR_OPTIONS = '--rsh-command=false';
  const result = spawnSync('bash', [...shell, command], {
    cwd: root, encoding: 'utf8', timeout: 5000,
    env,
  });
  assert.equal(result.error, undefined, result.error?.message);
  assert.equal(result.status, 0, result.stderr);
  const contents = gunzipSync(readFileSync(resolve(root, archive)));
  assert.equal(contents.subarray(257, 263).toString(), 'ustar\0');
  assert.equal(contents.subarray(263, 265).toString(), '00');
  assert.ok(contents.includes(Buffer.from('synthetic-native-bytes')));
  const hashCommand = workflow.split('\n').find(line => line.includes('file+".sha256"')
    && line.includes('path.basename(file)'))?.trim();
  assert.ok(hashCommand, 'Missing recovery archive checksum command');
  const hashResult = spawnSync('bash', [...shell, hashCommand], {
    cwd: root, encoding: 'utf8', timeout: 5000, env,
  });
  assert.equal(hashResult.error, undefined, hashResult.error?.message);
  assert.equal(hashResult.status, 0, hashResult.stderr);
  const sidecar = readFileSync(resolve(root, `${archive}.sha256`), 'utf8');
  const digest = createHash('sha256').update(readFileSync(resolve(root, archive))).digest('hex');
  assert.equal(sidecar, `${digest}  framer-next-windows-x64.tar.gz\n`);
});

test('server builds use only the typed server target and safe installs', () => {
  const root = JSON.parse(read('package.json'));
  const nativeTools = root.scripts['test:native-tools'];
  assert.match(nativeTools, /tsc -p tsconfig\.server-tools\.json/);
  assert.match(nativeTools, /test\/server-source-newlines\.test\.mjs/);
  assert.match(nativeTools, /test\/server-workflow\.test\.mjs/);
  assert.match(root.scripts['build:server'], /tsc -p framerize-server\/tsconfig.json/);
  assert.match(root.scripts['build:server'], /tsc -p tsconfig\.server-tools\.json/);
  const tools = JSON.parse(read('tsconfig.server-tools.json'));
  assert.deepEqual(tools.include, ['scripts/build-server.ts',
    'scripts/server-native-bundle.ts', 'scripts/native-account-smoke.ts',
    'scripts/package-framer-next.ts', 'test/framer-next.test.ts',
    'test/server-native-bundle.test.ts', 'test/server-native-account.test.ts']);
  assert.doesNotMatch(root.scripts['build:server'], /framerize-remote/);
  assert.equal((workflow.match(/npm run build:server --prefix \.\./g) ?? []).length, 3);
  assert.doesNotMatch(workflow, /npm ci\s*\n|npm run build --prefix/);
  assert.match(workflow, /NODE_OPTIONS: ''/);
  assert.equal((workflow.match(/persist-credentials: false/g) ?? []).length, 4);
  assert.match(workflow, /npm run test:native-tools --prefix \.\./);
});

test('server workflows run through the ci.yml gate and keep all pinned platforms', () => {
  const ci = read('.github/workflows/ci.yml');
  assert.match(workflow, /^on:\n  workflow_call:\n  workflow_dispatch:\n\n/m);
  assert.match(ci, /if: needs\.changes\.outputs\.server != 'false' && !inputs\.skip_server\n    uses: \.\/\.github\/workflows\/server-ci\.yml/);
  assert.match(read('.github/workflows/release.yml'), /with:\n      skip_server: true/);
  assert.match(workflow, /node: \['22\.12\.0', '22\.23\.4', '24\.13\.0'\]/);
  assert.match(workflow, /run: npm test -- --test-reporter=tap\n/);
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
  const { decide, readWorkspace } = createRequire(import.meta.url)('../.github/scripts/ci-changes.cjs');
  assert.equal(decide(['scripts/native-account-smoke.ts'], readWorkspace(), false).outputs.server, true);
});
