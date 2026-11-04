import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test, { type TestContext } from 'node:test';
import { prepareRemotePromotion } from '../scripts/package-promotion.js';

const hash = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex');
function validateWorkflow(source: string): void {
  const publish = source.replace(/\r\n/g, '\n').split(/^      - /m)
    .find(step => step.startsWith('name: Publish the exact approved tarball\n'));
  assert.ok(publish);
  assert.match(publish, /if: inputs.publish/);
  assert.ok(publish.indexOf('process.env.EXPECTED_INTEGRITY') < publish.indexOf('npm exec'));
  assert.match(source, /workflow_dispatch:/);
  const triggerSection = source.split('\npermissions:')[0]!;
  assert.deepEqual([...triggerSection.matchAll(/^  ([\w-]+):/gm)]
    .map(match => match[1]), ['workflow_dispatch']);
  assert.match(source, /if: github.ref == 'refs\/heads\/main'/);
  assert.match(source, /environment: npm-release/);
  assert.match(source, /default: false/);
  assert.match(source, /id-token: write/);
  assert.match(source, /node-version: '24.13.0'/);
  assert.match(source, /test "\$\(npm --version\)" = 11.6.2/);
  assert.match(source, /release-approvals\/framerize-remote.json/);
  assert.match(source, /npm publish "\$TARGET_TARBALL"/);
  assert.match(source, /--ignore-scripts --provenance=false --registry=https:\/\/registry.npmjs.org\//);
  assert.match(source, /npm exec --yes --ignore-scripts --package=/);
  assert.match(source, /process.env.EXPECTED_INTEGRITY/);
  assert.doesNotMatch(source, /secrets\.|npm (?:pack|run build)|build:native/);
}

test('publication workflow requires approval and an explicit publish choice', () => {
  const source = readFileSync(new URL('../../.github/workflows/publish-remote.yml', import.meta.url), 'utf8')
    .replace(/\r\n/g, '\n');
  validateWorkflow(source);
  assert.throws(() => validateWorkflow(source.replace('on:\n', 'on:\n  schedule:\n')));
  for (const token of ['environment: npm-release', 'default: false', '        if: inputs.publish',
    'release-approvals/framerize-remote.json', 'npm publish "$TARGET_TARBALL"', '--provenance=false']) {
    assert.throws(() => validateWorkflow(source.replace(token, 'removed')));
  }
});
const mutations: [string, (state: ReturnType<typeof fixture>) => void][] = [
  ['unapproved decision', state => { state.approval.decision = 'pending'; state.saveApproval(); }],
  ['missing approval', state => { rmSync(state.approvalFile); }],
  ['wrong approved source', state => { state.approval.sourceSha = 'd'.repeat(40); state.saveApproval(); }],
  ['wrong approved run', state => { state.approval.candidateRunId = '9999'; state.saveApproval(); }],
  ['wrong approved manifest', state => { state.approval.candidateManifestSha256 = 'e'.repeat(64); state.saveApproval(); }],
  ['missing reviewer', state => { state.approval.reviewedBy = ''; state.saveApproval(); }],
  ['evidence path escape', state => { state.approval.evidenceFile = '../private.md'; state.saveApproval(); }],
  ['changed live evidence', state => { writeFileSync(join(state.root, 'framerize-remote-live.md'), 'changed'); }],
  ['changed release', state => { writeFileSync(join(state.candidate, 'release.json'), '{}'); }],
  ['changed package', state => { writeFileSync(join(state.candidate, state.tarball), 'changed'); }],
  ['changed sums', state => { writeFileSync(join(state.candidate, 'SHA256SUMS'), 'changed'); }],
  ['extra candidate file', state => { writeFileSync(join(state.candidate, 'extra.txt'), 'extra'); }],
  ['wrong run', state => { state.expected.run.id = 9999; }],
  ['wrong workflow', state => { state.expected.run.path = '.github/workflows/other.yml'; }],
  ['failed workflow', state => { state.expected.run.conclusion = 'failure'; }],
  ['unfinished workflow', state => { state.expected.run.status = 'in_progress'; }],
  ['different attempt', state => { state.expected.run.run_attempt = 2; }],
  ['fork source', state => { state.expected.run.head_repository.full_name = 'fork/repository'; }],
  ['wrong repository', state => { state.expected.run.repository.full_name = 'other/repository'; }],
  ['pull request event', state => { state.expected.run.event = 'pull_request'; }],
  ['invalid approval commit', state => { state.expected.approvalCommit = 'unknown'; }],
  ['no environment protection', state => { state.expected.environment.protection_rules = []; }],
  ['empty environment reviewers', state => { state.expected.environment.protection_rules[0]!.reviewers = []; }],
  ['missing platform report', state => {
    rmSync(join(state.candidate, 'reports/windows-x64-node-24.13.0/report.json'));
  }],
  ['changed platform log', state => {
    writeFileSync(join(state.candidate, 'reports/linux-x64-node-22.12.0/installed.log'), 'changed');
  }],
];

for (const [name, mutate] of mutations) {
  test(`rejects ${name} before writing promotion output`, context => {
    const state = fixture(context); mutate(state);
    assert.throws(state.run);
    assert.equal(existsSync(state.output), false);
  });
}

function rebind(state: ReturnType<typeof fixture>): void {
  const data = Buffer.from(JSON.stringify(state.release));
  writeFileSync(join(state.candidate, 'release.json'), data);
  state.expected.releaseSha256 = hash(data);
  state.approval.candidateManifestSha256 = hash(data);
  state.saveApproval();
}

function repack(state: ReturnType<typeof fixture>, patch: Record<string, unknown>): void {
  const file = join(state.root, 'package/package.json');
  writeFileSync(file, JSON.stringify({ ...JSON.parse(readFileSync(file, 'utf8')), ...patch }));
  execFileSync('tar', ['--format=ustar', '-czf', state.tarball, 'package'], { cwd: state.root });
  const data = readFileSync(join(state.root, state.tarball));
  writeFileSync(join(state.candidate, state.tarball), data);
  const entry = state.release.evidence.find(item => item.file === state.tarball)!;
  entry.bytes = data.length; entry.sha256 = hash(data);
  rebind(state);
  const files = [...state.release.evidence.map(item => item.file), 'release.json'];
  writeFileSync(join(state.candidate, 'SHA256SUMS'), files.map(name =>
    `${hash(readFileSync(join(state.candidate, name)))}  ${name}\n`).join(''));
}

const manifestChanges = {
  name: { name: 'another-package' },
  version: { version: '9.9.9' },
  private: { private: true },
  registry: { publishConfig: { registry: 'https://example.invalid' } },
  dependencies: { dependencies: { unexpected: '1.0.0' } },
};
for (const [name, patch] of Object.entries(manifestChanges)) {
  test(`rejects approved package bytes with invalid ${name}`, context => {
    const state = fixture(context); repack(state, patch);
    assert.throws(state.run);
    assert.equal(existsSync(state.output), false);
  });
}

for (const change of ['path', 'duplicate', 'version', 'gate'] as const) {
  test(`rejects an approved manifest with invalid ${change}`, context => {
    const state = fixture(context);
    if (change === 'path') state.release.evidence[0]!.file = '../outside.tgz';
    if (change === 'duplicate') state.release.evidence.push(state.release.evidence[0]!);
    if (change === 'version') state.release.version = '../invalid';
    if (change === 'gate') state.release.pendingGate = 'none';
    rebind(state);
    assert.throws(state.run);
    assert.equal(existsSync(state.output), false);
  });
}

function fixture(context: TestContext, version = '0.2.0') {
  const root = mkdtempSync(join(tmpdir(), 'remote promotion '));
  context.after(() => rmSync(root, { recursive: true, force: true }));
  const candidate = join(root, 'candidate'); mkdirSync(candidate);
  const approvalFile = join(root, 'approval.json');
  const output = join(root, 'output');
  const packageFolder = join(root, 'package'); mkdirSync(packageFolder);
  writeFileSync(join(packageFolder, 'package.json'), JSON.stringify({ name: 'framerize-remote', version }));
  const tarball = `framerize-remote-${version}.tgz`;
  execFileSync('tar', ['--format=ustar', '-czf', tarball, 'package'], { cwd: root });
  const original = readFileSync(join(root, tarball));
  writeFileSync(join(candidate, tarball), original);
  const source = { repository: 'fixture/repository', sha: 'a'.repeat(40), tree: 'b'.repeat(40),
    event: 'push', runId: '1234', runAttempt: '1' };
  const expected = { repository: source.repository, runId: source.runId, releaseSha256: '',
    approvalCommit: 'c'.repeat(40),
    run: { id: 1234, run_attempt: 1, head_sha: source.sha, event: 'push', status: 'completed',
      conclusion: 'success', path: '.github/workflows/release.yml',
      repository: { full_name: source.repository }, head_repository: { full_name: source.repository } },
    environment: { protection_rules: [{ type: 'required_reviewers', reviewers: [{ type: 'User' }] }] } };
  const entries = new Map<string, Buffer>([[tarball, original], ['producer.json', Buffer.from('{}')]]);
  for (const target of ['linux-x64', 'macos-arm64', 'windows-x64']) {
    for (const node of ['22.12.0', '22.23.4', '24.13.0']) {
      for (const file of ['report.json', 'checker.json', 'installed.log']) {
        entries.set(`reports/${target}-node-${node}/${file}`, Buffer.from('synthetic retained evidence'));
      }
    }
  }
  const release = { name: 'framerize-remote', version, tag: `framerize-remote-v${version}`,
    source, assembledAttempt: '1', publishable: false, pendingGate: 'live_browserless_qualification',
    evidence: [...entries].map(([file, data]) => ({ file, bytes: data.length, sha256: hash(data) })) };
  const releaseBytes = Buffer.from(JSON.stringify(release));
  expected.releaseSha256 = hash(releaseBytes);
  entries.set('release.json', releaseBytes);
  for (const [file, data] of entries) {
    mkdirSync(dirname(join(candidate, file)), { recursive: true });
    writeFileSync(join(candidate, file), data);
  }
  writeFileSync(join(candidate, 'SHA256SUMS'), [...entries].map(([file, data]) => `${hash(data)}  ${file}\n`).join(''));
  const evidence = Buffer.from('Synthetic review fixture, not a live inference result.');
  writeFileSync(join(root, 'framerize-remote-live.md'), evidence);
  const approval = { schemaVersion: 1, decision: 'approve-npm-publication', candidateRunId: '1234',
    candidateManifestSha256: expected.releaseSha256, sourceSha: source.sha, reviewedBy: 'fixture-owner',
    evidenceFile: 'framerize-remote-live.md', evidenceSha256: hash(evidence) };
  const saveApproval = () => writeFileSync(approvalFile, JSON.stringify(approval));
  saveApproval();
  return { root, candidate, approvalFile, output, expected, approval, release, original, tarball, saveApproval,
    run: () => prepareRemotePromotion(candidate, approvalFile, output, expected) };
}

for (const version of ['0.2.0', '0.2.0-rc.1']) {
  test(`prepares exact approved bytes for ${version} without publishing`, context => {
    const state = fixture(context, version);
    const result = state.run();
    assert.deepEqual(readFileSync(result.tarball), state.original);
    assert.equal(result.sourceSha, state.expected.run.head_sha);
    assert.equal(result.approvalCommit, state.expected.approvalCommit);
    assert.equal(result.distTag, version.includes('-') ? 'next' : 'latest');
    assert.equal(result.integrity, 'sha512-' + createHash('sha512').update(state.original).digest('base64'));
    assert.deepEqual(JSON.parse(readFileSync(join(state.output, 'promotion.json'), 'utf8')), result);
  });
}
