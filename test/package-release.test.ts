import test, { type TestContext } from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { assemblePackageRelease } from '../scripts/package-release.js';
import { readPackageArchive } from '../scripts/package-archive.js';
const hash = (data: Buffer) => createHash('sha256').update(data).digest('hex');
const mutations: [string, (state: ReturnType<typeof fixture>) => void][] = [
  ['tag', state => { state.expected.tag = 'framerize-remote-v9.9.9'; }],
  ['source tree', state => { state.expected.tree = 'd'.repeat(40); }],
  ['run', state => { state.expected.runId = '9876'; }],
  ['missing report', state => { rmSync(state.cells[0]!.folder, { recursive: true }); }],
  ['archive', state => { writeFileSync(join(state.input, 'framerize-remote-0.2.0.tgz'), 'changed'); }],
  ['runtime', state => { state.cells[0]!.report.node = 'v20.0.0'; }],
  ['architecture', state => { state.cells[0]!.report.arch = 'arm64'; }],
  ['future attempt', state => { state.cells[0]!.report.source.runAttempt = '3'; }],
  ['producer binding', state => { state.cells[0]!.report.producer.manifestSha256 = 'e'.repeat(64); }],
  ['package version', state => { state.cells[0]!.report.packages[0]!.version = '9.9.9'; }],
  ['inventory', state => { state.cells[0]!.report.packages[0]!.inventory[0]!.sha256 = 'f'.repeat(64); }],
  ['installed log', state => { writeFileSync(join(state.cells[0]!.folder, 'installed.log'), 'changed'); }],
];
for (const [name, modify] of mutations) {
  test(`rejects changed ${name} before writing release output`, context => {
    const state = fixture(context); modify(state);
    for (const cell of state.cells) if (existsSync(cell.folder)) cell.save();
    assert.throws(state.run);
    assert.equal(existsSync(state.output), false);
  });
}
test('promotes original bytes only after all nine offline reports, without claiming live acceptance', context => {
  const state = fixture(context);
  const original = readFileSync(join(state.input, 'framerize-remote-0.2.0.tgz'));
  const release = state.run();
  assert.equal(release.publishable, false);
  assert.equal(release.pendingGate, 'live_browserless_qualification');
  assert.equal(release.evidence.length, 29);
  assert.deepEqual(readFileSync(join(state.output, 'framerize-remote-0.2.0.tgz')), original);
  assert.equal(existsSync(join(state.output, 'framerize-server-0.1.0.tgz')), false);
});
function fixture(context: TestContext) {
  const root = mkdtempSync(join(tmpdir(), 'package release '));
  context.after(() => rmSync(root, { recursive: true, force: true }));
  const input = join(root, 'input'); const reports = join(root, 'reports');
  const output = join(root, 'output');
  mkdirSync(input); mkdirSync(reports); mkdirSync(join(input, 'tooling'));
  const expected = { repository: 'fixture/repository', sha: 'a'.repeat(40), tree: 'b'.repeat(40),
    event: 'push', runId: '1234', runAttempt: '2', tag: 'framerize-remote-v0.2.0',
    versions: { 'framerize-remote': '0.2.0', 'framerize-server': '0.1.0' } };
  const packages = Object.entries(expected.versions).map(([name, version]) => {
    const directory = join(root, name); mkdirSync(join(directory, 'package'), { recursive: true });
    writeFileSync(join(directory, 'package/package.json'), JSON.stringify({ name, version }));
    const tarball = `${name}-${version}.tgz`; const archive = join(input, tarball);
    execFileSync('tar', ['--format=ustar', '-czf', tarball, 'package'], { cwd: directory });
    const data = readFileSync(join(directory, tarball));
    writeFileSync(archive, data);
    return { name, version, tarball, bytes: data.length, sha256: hash(data) };
  });
  const source = { repository: expected.repository, sha: expected.sha, tree: expected.tree,
    event: expected.event, runId: expected.runId, runAttempt: '1' };
  const producer = { source, node: 'v22.23.4', npm: '10.9.9', packages };
  const manifest = Buffer.from(JSON.stringify(producer));
  writeFileSync(join(input, 'manifest.json'), manifest);
  const checked = packages.map(({ bytes: unused, ...entry }) => {
    const inventory = [...readPackageArchive(join(input, entry.tarball))].map(([path, data]) =>
      ({ path, bytes: data.length, sha256: hash(data) }));
    return { ...entry, files: inventory.length, inventory };
  });
  const platforms = [['linux-x64', 'linux', 'x64', 'ubuntu-22.04'],
    ['macos-arm64', 'darwin', 'arm64', 'macos-15'], ['windows-x64', 'win32', 'x64', 'windows-2022']] as const;
  const cells = platforms.flatMap(([target, platform, arch, runner]) =>
    ['22.12.0', '22.23.4', '24.13.0'].map(node => {
      const folder = join(reports, `package-coexistence-1234-report-${target}-node-${node}`);
      mkdirSync(folder);
      const log = Buffer.from('synthetic installed check');
      const checker = { node: `v${node}`, npm: '10.9.9', platform, arch, packages: checked };
      const report = { ...checker, target, runner, source: { ...source, runAttempt: '2' },
        producer: { source, manifestSha256: hash(manifest) }, installedLogSha256: hash(log) };
      writeFileSync(join(folder, 'installed.log'), log);
      const save = () => {
        writeFileSync(join(folder, 'report.json'), JSON.stringify(report));
        writeFileSync(join(folder, 'checker.json'), JSON.stringify(checker));
      };
      save(); return { folder, report, checker, save };
    }));
  return { input, reports, output, expected, producer, cells,
    run: () => assemblePackageRelease(input, reports, output, expected) };
}
