import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync,
  rmSync, statSync, truncateSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test, { type TestContext } from 'node:test';
import { createFramerNext } from '../scripts/package-framer-next.js';

const digest = (bytes: Buffer | string) => createHash('sha256').update(bytes).digest('hex');
const recoveryArchiveStep =
  'tar --format=ustar -czf - -C "$RUNNER_TEMP" framer-next > "$RECOVERY_ARCHIVE"';
test('native CI assembles recovery only after verification and preserves executable permissions in tar', () => {
  const source = readFileSync(new URL('../../.github/workflows/server-ci.yml', import.meta.url), 'utf8');
  const validate = (workflow: string) => {
    const metadata = workflow.indexOf('node scripts/release-metadata.js');
    const recovery = workflow.indexOf('node ../.tools/scripts/package-framer-next.js');
    const archive = workflow.indexOf(recoveryArchiveStep);
    assert.ok(metadata >= 0 && recovery > metadata && archive > recovery);
    assert.doesNotMatch(workflow, /tar --format=ustar -czf "\$RECOVERY_ARCHIVE"/);
    assert.match(workflow, /name: framer-next-\$\{\{ matrix.target \}\}/);
    assert.match(workflow, /framer-next-\$\{\{ matrix.target \}\}.tar.gz.sha256/);
  };
  validate(source);
  for (const token of ['node scripts/release-metadata.js', 'node ../.tools/scripts/package-framer-next.js',
    recoveryArchiveStep]) assert.throws(() => validate(source.replace(token, 'removed')));
});
const mutations: [string, (state: ReturnType<typeof fixture>) => void][] = [
  ['name', state => { state.manifest.name = 'other'; }],
  ['target', state => { state.manifest.target = 'unknown'; }],
  ['commit', state => { state.manifest.commit = 'not-a-commit'; }],
  ['Node version', state => { state.manifest.node = 'latest'; }],
  ['version path', state => { state.manifest.version = '../private'; }],
  ['package hash', state => { state.manifest.packageSha256 = 'd'.repeat(64); }],
  ['package size', state => { state.manifest.packageBytes = 0; }],
  ['unexpected artifact', state => { state.manifest.artifacts['../private'] = 'e'.repeat(64); }],
  ['binary hash', state => { writeFileSync(join(state.input, state.executable), 'changed'); }],
  ['license hash', state => { writeFileSync(join(state.input, 'LICENSE'), 'changed'); }],
  ['missing license', state => { rmSync(join(state.input, 'NODE-LICENSE')); }],
  ['binary size cap', state => { truncateSync(join(state.input, state.executable), 256 * 1024 * 1024 + 1); }],
];
for (const [name, mutate] of mutations) {
  test(`rejects invalid ${name} before creating a recovery folder`, context => {
    const state = fixture(context); mutate(state); state.save();
    assert.throws(state.run);
    assert.equal(existsSync(state.output), false);
  });
}

test('an untrusted manifest pin fails without creating output', context => {
  const state = fixture(context);
  assert.throws(() => createFramerNext(state.input, 'f'.repeat(64), state.output));
  assert.equal(existsSync(state.output), false);
});

test('existing instances are never overwritten', context => {
  const state = fixture(context); mkdirSync(state.output);
  writeFileSync(join(state.output, 'accounts.json'), 'keep-private-state');
  assert.throws(state.run);
  assert.deepEqual(readdirSync(state.output), ['accounts.json']);
  assert.equal(readFileSync(join(state.output, 'accounts.json'), 'utf8'), 'keep-private-state');
});

test('identical inputs produce identical recovery file bytes in different folders', context => {
  const state = fixture(context); state.run();
  const second = state.output + ' copy # %';
  createFramerNext(state.input, digest(readFileSync(join(state.input, 'release.json'))), second);
  for (const name of readdirSync(state.output)) {
    assert.deepEqual(readFileSync(join(second, name)), readFileSync(join(state.output, name)));
  }
});

test('CLI help is inert and errors omit supplied arguments', context => {
  const state = fixture(context);
  const script = fileURLToPath(new URL('../scripts/package-framer-next.js', import.meta.url));
  const env = { HOME: state.input, TMPDIR: state.input, NODE_OPTIONS: '', NODE_PATH: '',
    SystemRoot: process.env.SystemRoot, WINDIR: process.env.WINDIR };
  for (const args of [['--help'], ['synthetic-private-marker']]) {
    const result = spawnSync(process.execPath, [script, ...args], { env, encoding: 'utf8', timeout: 5000 });
    assert.equal(result.error, undefined);
    assert.equal(result.status, args[0] === '--help' ? 0 : 1);
    assert.doesNotMatch(result.stdout + result.stderr, /synthetic-private-marker/);
    assert.equal(existsSync(state.output), false);
  }
});
function fixture(context: TestContext, target = 'linux-x64') {
  const root = mkdtempSync(join(tmpdir(), 'framer next '));
  context.after(() => rmSync(root, { recursive: true, force: true }));
  const input = join(root, 'input'); const output = join(root, 'Framer Next');
  mkdirSync(input);
  const executable = 'framerize-server' + (target === 'windows-x64' ? '.exe' : '');
  const archive = `framerize-server-${target}.${target === 'windows-x64' ? 'zip' : 'tar.gz'}`;
  const artifacts: Record<string, string> = { [archive]: 'b'.repeat(64),
    'framerize-0.1.0.tgz': 'c'.repeat(64) };
  for (const name of [executable, 'LICENSE', 'NODE-LICENSE']) {
    const content = 'synthetic ' + name;
    writeFileSync(join(input, name), content); artifacts[name] = digest(content);
  }
  const manifest = { name: 'framerize', version: '0.1.0', commit: 'a'.repeat(40),
    target, node: 'v22.23.4', packageBytes: 100, packageSha256: 'c'.repeat(64), artifacts };
  let pin = '';
  const save = () => {
    const data = JSON.stringify(manifest);
    writeFileSync(join(input, 'release.json'), data); pin = digest(data);
  };
  save();
  return { input, output, executable, manifest, save, run: () => createFramerNext(input, pin, output) };
}

for (const target of ['linux-x64', 'macos-arm64', 'windows-x64']) {
  test(`creates the exact credential-free recovery layout for ${target}`, context => {
    const state = fixture(context, target);
    writeFileSync(join(state.input, 'accounts.json'), 'synthetic-private-marker');
    const result = state.run();
    assert.equal(result.name, 'Framer Next');
    assert.equal(result.sourceCommit, state.manifest.commit);
    assert.deepEqual(readdirSync(state.output).sort(), [state.executable, 'LICENSE', 'NODE-LICENSE',
      'accounts.example.json', 'README.md', 'native-release.json', 'recovery.json', 'SHA256SUMS'].sort());
    assert.deepEqual(readFileSync(join(state.output, state.executable)), readFileSync(join(state.input, state.executable)));
    assert.deepEqual(readFileSync(join(state.output, 'native-release.json')), readFileSync(join(state.input, 'release.json')));
    const account = JSON.parse(readFileSync(join(state.output, 'accounts.example.json'), 'utf8'))[0];
    assert.equal(account.enabled, false);
    assert.deepEqual(account.session, { file: 'private/session' });
    const sums = readFileSync(join(state.output, 'SHA256SUMS'), 'utf8').trimEnd().split('\n');
    assert.equal(sums.length, 7);
    for (const line of sums) {
      const [hash, name] = line.split('  ');
      assert.equal(digest(readFileSync(join(state.output, name!))), hash);
    }
    for (const name of ['README.md', 'accounts.example.json', 'recovery.json']) {
      assert.doesNotMatch(readFileSync(join(state.output, name), 'utf8'), /synthetic-private-marker/);
    }
    if (process.platform !== 'win32') {
      assert.equal(statSync(join(state.output, state.executable)).mode & 0o777, 0o755);
      assert.equal(statSync(state.output).mode & 0o777, 0o700);
      assert.equal(statSync(join(state.output, 'accounts.example.json')).mode & 0o777, 0o600);
    }
  });
}
