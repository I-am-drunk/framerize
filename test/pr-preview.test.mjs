import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
const require = createRequire(import.meta.url);
const { context, versionFor, stamp, exportPreview } = require('../.github/scripts/preview-version.cjs');
const { validatePreview, commentBody, hash } = require('../.github/scripts/pr-preview-manifest.cjs');
const { belongsToPr } = require('../.github/scripts/publish-pr-preview.cjs');
const environment = { GITHUB_EVENT_NAME: 'pull_request', GITHUB_REPOSITORY: 'owner/project',
  FRAMERIZE_PR_NUMBER: '23', FRAMERIZE_PR_HEAD_SHA: 'a'.repeat(40),
  GITHUB_SHA: 'b'.repeat(40), GITHUB_RUN_ID: '12345', GITHUB_RUN_ATTEMPT: '1' };
function temporary(t) {
  const root = mkdtempSync(join(tmpdir(), 'framerize-preview-test-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return root;
}

test('PR versions identify the built merge and remain stable across attempts', () => {
  const info = context(environment);
  assert.equal(info.headSha, 'a'.repeat(40));
  assert.equal(info.buildSha, 'b'.repeat(40));
  assert.equal(versionFor('1.2.3-alpha.1', info), '1.2.3-pr.23.12345.gbbbbbbbbbbbb');
  assert.equal(versionFor('1.2.3', { ...info, attempt: '2' }), versionFor('1.2.3', info));
  assert.notEqual(versionFor('1.2.3', { ...info, runId: '12346' }), versionFor('1.2.3', info));
  for (const value of ['01.2.3', '../1.2.3', '1.2', '1.2.3;bad']) {
    assert.throws(() => versionFor(value, info));
  }
  for (const key of Object.keys(environment).filter(key => key !== 'GITHUB_EVENT_NAME')) {
    assert.throws(() => context({ ...environment, [key]: 'bad/input\n' }), key);
  }
});

test('stamping changes both package versions without changing stable builds', t => {
  const root = temporary(t);
  for (const [dir, name] of [['framerize-server', 'framerize'], ['framerize-remote', 'framerize-remote']]) {
    mkdirSync(join(root, dir));
    writeFileSync(join(root, dir, 'package.json'), JSON.stringify({ name, version: '1.2.3', files: ['dist'] }));
  }
  const file = join(root, 'framerize-server/package.json');
  const original = readFileSync(file);
  assert.equal(stamp(root, { GITHUB_EVENT_NAME: 'push' }), null);
  assert.deepEqual(readFileSync(file), original);
  assert.equal(stamp(root, environment).length, 2);
  const first = readFileSync(file);
  stamp(root, { ...environment, GITHUB_RUN_ATTEMPT: '2' });
  assert.deepEqual(readFileSync(file), first);
  assert.deepEqual(JSON.parse(first).files, ['dist']);
});

function previewFixture(t) {
  const root = temporary(t);
  const input = join(root, 'input');
  const output = join(root, 'preview');
  mkdirSync(input);
  const info = context(environment);
  const packages = ['framerize-remote', 'framerize'].map(name => {
    const version = versionFor('1.2.3', info);
    const tarball = `${name}-${version}.tgz`;
    const bytes = Buffer.from(`synthetic archive ${name}`);
    writeFileSync(join(input, tarball), bytes);
    return { name, version, tarball, sha256: hash(bytes), bytes: bytes.length };
  });
  const producer = { source: { repository: info.repository, sha: info.buildSha,
    tree: 'c'.repeat(40), event: 'pull_request', runId: info.runId, runAttempt: info.attempt },
    packages, node: 'v22.23.4', npm: '10.9.4', tooling: [] };
  writeFileSync(join(input, 'manifest.json'), JSON.stringify(producer));
  const run = { id: 12345, head_sha: info.headSha, run_attempt: 2,
    repository: { full_name: info.repository } };
  const pr = { number: 23, head: { sha: info.headSha }, merge_commit_sha: info.buildSha };
  const commit = { sha: info.buildSha, commit: { tree: { sha: producer.source.tree } },
    parents: [{ sha: 'd'.repeat(40) }, { sha: info.headSha }] };
  return { input, output, run, pr, commit, packages };
}

test('preview export preserves package bytes and records both commits', t => {
  const f = previewFixture(t);
  const manifest = exportPreview(f.input, f.output, environment);
  const checked = validatePreview(f.output, f.run, f.pr, f.commit);
  assert.equal(checked.files.size, 5);
  assert.deepEqual(checked.manifest, manifest);
  for (const entry of f.packages) {
    assert.deepEqual(readFileSync(join(f.input, entry.tarball)), checked.files.get(entry.tarball));
  }
  assert.equal(readdirSync(f.output).length, 4);
  const urls = new Map([...checked.files].map(([name]) => [name, `https://example.com/${name}`]));
  const message = commentBody(manifest, urls, checked.prefix);
  assert.match(message, /Tested merge commit: `bbbb/);
  assert.match(message, /PR head: `aaaa/);
  assert.match(message, /--prefix "\$preview\/server"/);
  assert.match(message, /--prefix "\$preview\/remote"/);
  assert.doesNotMatch(message, /service uninstall|npm publish/);
});

test('preview validation refuses a moved PR, wrong merge, future attempt and changed bytes', t => {
  const f = previewFixture(t);
  exportPreview(f.input, f.output, environment);
  assert.throws(() => validatePreview(f.output, f.run, { ...f.pr, head: { sha: 'e'.repeat(40) } }, f.commit));
  assert.throws(() => validatePreview(f.output, f.run,
    { ...f.pr, merge_commit_sha: 'f'.repeat(40) }, f.commit));
  assert.throws(() => validatePreview(f.output, { ...f.run, run_attempt: 0 }, f.pr, f.commit));
  assert.throws(() => validatePreview(f.output, f.run, f.pr,
    { ...f.commit, commit: { tree: { sha: 'f'.repeat(40) } } }));
  writeFileSync(join(f.output, f.packages[0].tarball), 'changed archive');
  assert.throws(() => validatePreview(f.output, f.run, f.pr, f.commit));
});

test('PR cleanup selects only that PR and recognized preview asset names', () => {
  for (const name of ['framerize-1.2.3-pr.23.12345.gbbbbbbbbbbbb.tgz',
    'framerize-remote-1.2.3-pr.23.12345.gbbbbbbbbbbbb.tgz', 'pr-23-12345-1-SHA256SUMS']) {
    assert.equal(belongsToPr(name, 23), true);
    assert.equal(belongsToPr(name, 2), false);
  }
  for (const name of ['framerize-1.2.3.tgz', 'pr-23-release.json', 'pr-230-12345-1-preview.json']) {
    assert.equal(belongsToPr(name, 23), false);
  }
});
