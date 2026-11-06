import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { appendFileSync, lstatSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { readPackageArchive } from './package-archive.js';

const digest = (data: Buffer) => createHash('sha256').update(data).digest('hex');
const hashPattern = /^[a-f0-9]{64}$/;
interface Entry { file: string; bytes: number; sha256: string }
interface Candidate {
  name: string; version: string; tag: string; assembledAttempt: string;
  publishable: boolean; pendingGate: string; evidence: Entry[];
  source: { repository: string; sha: string; runId: string; event: string };
}
interface Approval {
  schemaVersion: number; decision: string; candidateRunId: string;
  candidateManifestSha256: string; sourceSha: string; reviewedBy: string;
  evidenceFile: string; evidenceSha256: string;
}
interface Run {
  id: number; run_attempt: number; head_sha: string; event: string; status: string;
  conclusion: string; path: string; repository: { full_name: string };
  head_repository: { full_name: string };
}
interface Environment { protection_rules: { type: string; reviewers?: unknown[] }[] }
export interface PromotionContext {
  repository: string; runId: string; releaseSha256: string; run: Run; environment: Environment;
  approvalCommit: string;
}
function bytes(file: string, limit = 2 * 1024 * 1024): Buffer {
  const info = lstatSync(file);
  assert.ok(info.isFile() && info.size > 0 && info.size <= limit, 'Invalid promotion input');
  return readFileSync(file);
}
function files(root: string, prefix = ''): string[] {
  assert.ok(lstatSync(root).isDirectory(), 'Invalid candidate directory');
  assert.ok(prefix.split('/').length <= 3, 'Invalid candidate depth');
  const names = readdirSync(root); assert.ok(names.length <= 32, 'Too many candidate entries');
  return names.flatMap(name => lstatSync(join(root, name)).isDirectory()
    ? files(join(root, name), prefix + name + '/') : [prefix + name]);
}

export function prepareRemotePromotion(candidate: string, approvalFile: string,
  output: string, expected: PromotionContext) {
  assert.match(expected.runId, /^[1-9]\d*$/);
  assert.match(expected.releaseSha256, hashPattern);
  const run = expected.run;
  assert.equal(String(run.id), expected.runId);
  assert.equal(run.repository.full_name, expected.repository);
  assert.equal(run.head_repository.full_name, expected.repository);
  assert.equal(run.path, '.github/workflows/release.yml');
  assert.equal(run.event, 'push'); assert.equal(run.status, 'completed');
  assert.equal(run.conclusion, 'success'); assert.match(run.head_sha, /^[a-f0-9]{40}$/);
  assert.match(expected.approvalCommit, /^[a-f0-9]{40}$/);
  assert.ok(Number.isSafeInteger(run.run_attempt) && run.run_attempt > 0);
  assert.ok(expected.environment.protection_rules.some(rule =>
    rule.type === 'required_reviewers' && Array.isArray(rule.reviewers) && rule.reviewers.length > 0),
  'Publishing requires environment reviewers');
  const releaseBytes = bytes(join(candidate, 'release.json'));
  assert.equal(digest(releaseBytes), expected.releaseSha256);
  const release = JSON.parse(releaseBytes.toString('utf8')) as Candidate;
  assert.equal(release.name, 'framerize-remote');
  assert.match(release.version, /^\d+\.\d+\.\d+(?:-[\w.-]+)?(?:\+[\w.-]+)?$/);
  assert.equal(release.tag, `framerize-remote-v${release.version}`);
  assert.equal(release.source.repository, expected.repository);
  assert.equal(release.source.sha, run.head_sha);
  assert.equal(release.source.runId, expected.runId);
  assert.equal(release.source.event, 'push');
  assert.equal(release.assembledAttempt, String(run.run_attempt));
  assert.equal(release.publishable, false);
  assert.equal(release.pendingGate, 'live_browserless_qualification');
  const approvalBytes = bytes(approvalFile, 65536);
  const approval = JSON.parse(approvalBytes.toString('utf8')) as Approval;
  assert.equal(approval.schemaVersion, 1);
  assert.equal(approval.decision, 'approve-npm-publication');
  assert.equal(approval.candidateRunId, expected.runId);
  assert.equal(approval.candidateManifestSha256, expected.releaseSha256);
  assert.equal(approval.sourceSha, run.head_sha);
  assert.match(approval.reviewedBy, /^[A-Za-z0-9][A-Za-z0-9-]{0,38}$/);
  assert.equal(approval.evidenceFile, 'framerize-remote-live.md');
  assert.match(approval.evidenceSha256, hashPattern);
  assert.equal(digest(bytes(join(dirname(approvalFile), approval.evidenceFile), 262144)),
    approval.evidenceSha256, 'Live review evidence differs');
  const tarball = `framerize-remote-${release.version}.tgz`;
  const reportFiles = ['linux-x64', 'macos-arm64', 'windows-x64'].flatMap(target =>
    ['22.12.0', '22.23.4', '24.13.0'].flatMap(node =>
      ['report.json', 'checker.json', 'installed.log'].map(file => `reports/${target}-node-${node}/${file}`)));
  const evidence: Entry[] = release.evidence;
  assert.deepEqual(evidence.map(entry => entry.file).sort(), [tarball, 'producer.json', ...reportFiles].sort());
  assert.deepEqual(files(candidate).sort(), [...evidence.map(entry => entry.file), 'release.json', 'SHA256SUMS'].sort());
  const snapshots = new Map<string, Buffer>();
  for (const entry of evidence) {
    assert.match(entry.sha256, hashPattern);
    const data = bytes(join(candidate, entry.file),
      entry.file.endsWith('/installed.log') ? 16 * 1024 * 1024 : undefined);
    assert.equal(data.length, entry.bytes); assert.equal(digest(data), entry.sha256);
    snapshots.set(entry.file, data);
  }
  snapshots.set('release.json', releaseBytes);
  const sums = [...snapshots].map(([file, data]) => `${digest(data)}  ${file}\n`).join('');
  assert.equal(bytes(join(candidate, 'SHA256SUMS')).toString('utf8'), sums);
  const archive = snapshots.get(tarball)!;
  const manifest = JSON.parse(readPackageArchive(archive).get('package/package.json')!.toString('utf8'));
  assert.equal(manifest.name, 'framerize-remote'); assert.equal(manifest.version, release.version);
  assert.notEqual(manifest.private, true);
  assert.equal(manifest.publishConfig, undefined);
  for (const key of ['dependencies', 'optionalDependencies', 'peerDependencies']) {
    assert.equal(Object.keys(manifest[key] ?? {}).length, 0, 'Remote must have no runtime dependencies');
  }
  const result = { tarball: join(resolve(output), tarball), version: release.version,
    integrity: 'sha512-' + createHash('sha512').update(archive).digest('base64'),
    distTag: release.version.split('+', 1)[0]!.includes('-') ? 'next' : 'latest', sha256: digest(archive),
    sourceSha: run.head_sha, candidateRunId: expected.runId, candidateManifestSha256: digest(releaseBytes),
    approvalCommit: expected.approvalCommit, approvalSha256: digest(approvalBytes),
    evidenceSha256: approval.evidenceSha256, reviewedBy: approval.reviewedBy };
  mkdirSync(output, { mode: 0o700 });
  writeFileSync(result.tarball, archive, { flag: 'wx', mode: 0o600 });
  writeFileSync(join(output, 'promotion.json'), JSON.stringify(result, null, 2) + '\n', { flag: 'wx' });
  return result;
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    assert.equal(process.argv.length, 7);
    assert.equal(process.env.GITHUB_EVENT_NAME, 'workflow_dispatch');
    assert.equal(process.env.GITHUB_REF, 'refs/heads/main');
    const head = execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim();
    assert.equal(head, process.env.GITHUB_SHA);
    const result = prepareRemotePromotion(process.argv[2]!, process.argv[3]!, process.argv[4]!, {
      repository: process.env.GITHUB_REPOSITORY!, runId: process.env.CANDIDATE_RUN!,
      releaseSha256: process.env.CANDIDATE_MANIFEST_SHA256!, approvalCommit: head,
      run: JSON.parse(bytes(process.argv[5]!).toString('utf8')),
      environment: JSON.parse(bytes(process.argv[6]!).toString('utf8')),
    });
    assert.doesNotMatch(result.tarball, /[\r\n]/);
    if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT,
      `tarball=${result.tarball}\ndist-tag=${result.distTag}\nversion=${result.version}\nintegrity=${result.integrity}\n`);
    console.log(JSON.stringify(result));
  } catch {
    console.error('Remote publication qualification failed'); process.exitCode = 1;
  }
}
