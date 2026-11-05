import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { execFileSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, readdirSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import nodeTest from 'node:test';
// The publisher and its Python staging process run only on Ubuntu.
const test = process.platform === 'linux' ? nodeTest : nodeTest.skip;
const require = createRequire(import.meta.url);
const { runPreview, upload, updateComment } = require('../.github/scripts/publish-pr-preview.cjs');
const { hash, marker } = require('../.github/scripts/pr-preview-manifest.cjs');
const { exportPreview, context, versionFor } = require('../.github/scripts/preview-version.cjs');
const repo = { owner: 'owner', repo: 'project' };
const env = { GITHUB_EVENT_NAME: 'pull_request', GITHUB_REPOSITORY: 'owner/project',
  FRAMERIZE_PR_NUMBER: '23', FRAMERIZE_PR_HEAD_SHA: 'a'.repeat(40),
  GITHUB_SHA: 'b'.repeat(40), GITHUB_RUN_ID: '12345', GITHUB_RUN_ATTEMPT: '1' };

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'framerize-publisher-test-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const input = join(root, 'input');
  mkdirSync(input);
  const info = context(env);
  const packages = ['framerize-remote', 'framerize'].map(name => {
    const version = versionFor('1.2.3', info);
    const tarball = `${name}-${version}.tgz`;
    const bytes = Buffer.from(`test archive ${name}`);
    writeFileSync(join(input, tarball), bytes);
    return { name, version, tarball, bytes: bytes.length, sha256: hash(bytes) };
  });
  const producer = { source: { repository: info.repository, sha: info.buildSha,
    tree: 'c'.repeat(40), event: 'pull_request', runId: info.runId, runAttempt: '1' },
    node: 'v22.23.4', npm: '10.9.4', packages };
  writeFileSync(join(input, 'manifest.json'), JSON.stringify(producer));
  const staged = join(root, 'staged');
  exportPreview(input, staged, env);
  const zip = join(root, 'preview.zip');
  const python = 'import sys,zipfile,pathlib\nwith zipfile.ZipFile(sys.argv[1],"w") as z:\n for p in pathlib.Path(sys.argv[2]).iterdir(): z.write(p,p.name)';
  execFileSync('python3', ['-c', python, zip, staged], { timeout: 10_000 });
  return mock(readFileSync(zip), packages);
}

function mock(archive, packages) {
  const pr = { number: 23, state: 'open', head: { sha: env.FRAMERIZE_PR_HEAD_SHA },
    base: { repo: { id: 1 } }, merge_commit_sha: env.GITHUB_SHA };
  const run = { id: 12345, event: 'pull_request', status: 'completed', conclusion: 'success',
    path: '.github/workflows/ci.yml', head_sha: pr.head.sha, run_attempt: 2,
    repository: { id: 1, full_name: 'owner/project' }, pull_requests: [pr] };
  const state = { pr, run, packages, assets: [], comments: [], writes: [], logs: [],
    jobs: [{ name: 'Check', conclusion: 'success' }], onUpload() {}, onDownload() {} };
  const github = { paginate: (method, args) => method(args), rest: { actions: {
    getWorkflowRun: async () => ({ data: structuredClone(state.run) }),
    listJobsForWorkflowRun: async () => state.jobs,
    listWorkflowRunArtifacts: async () => [{ name: 'pr-preview-12345', id: 5, size_in_bytes: archive.length }],
    downloadArtifact: async () => { state.onDownload(); return { data: archive }; },
  }, pulls: { get: async () => ({ data: structuredClone(state.pr) }) }, repos: {
    getCommit: async () => ({ data: { sha: env.GITHUB_SHA,
      commit: { tree: { sha: 'c'.repeat(40) } },
      parents: [{ sha: 'd'.repeat(40) }, { sha: env.FRAMERIZE_PR_HEAD_SHA }] } }),
    getReleaseByTag: async () => ({ data: { id: 10 } }),
    listReleaseAssets: async () => state.assets,
    uploadReleaseAsset: async ({ name, data }) => {
      const asset = { id: state.assets.length + 1, name, size: data.length,
        digest: `sha256:${hash(data)}`, browser_download_url: `https://example.com/${name}` };
      state.assets.push(asset);
      state.writes.push(['upload', name]);
      state.onUpload();
      return { data: asset };
    },
    deleteReleaseAsset: async ({ asset_id }) => {
      state.assets = state.assets.filter(asset => asset.id !== asset_id);
      state.writes.push(['delete', asset_id]);
    },
  }, issues: {
    listComments: async () => state.comments,
    createComment: async ({ body }) => {
      state.comments.push({ id: 1, body, user: { login: 'github-actions[bot]' } });
      state.writes.push(['comment-create']);
    },
    updateComment: async ({ comment_id, body }) => {
      state.comments.find(comment => comment.id === comment_id).body = body;
      state.writes.push(['comment-update']);
    },
  } } };
  const core = { info: message => state.logs.push(message) };
  return { github, state, core };
}

test('the complete publisher stages data, reuses immutable assets and updates one comment', async t => {
  const f = fixture(t);
  const temporary = () => readdirSync(tmpdir()).filter(name => name.startsWith('framerize-publish-')).sort();
  const before = temporary();
  await runPreview(f.github, repo, 12345, f.core);
  assert.equal(f.state.assets.length, 5);
  assert.equal(f.state.comments.length, 1);
  assert.match(f.state.comments[0].body, /Tested merge commit/);
  await runPreview(f.github, repo, 12345, f.core);
  assert.equal(f.state.assets.length, 5);
  assert.equal(f.state.comments.length, 1);
  assert.equal(f.state.writes.filter(w => w[0] === 'upload').length, 5);
  assert.equal(f.state.writes.filter(w => w[0] === 'comment-update').length, 1);
  assert.deepEqual(temporary(), before);
});

test('a moved PR and a non-successful Check cannot publish', async t => {
  const f = fixture(t);
  f.state.pr.head.sha = 'e'.repeat(40);
  await runPreview(f.github, repo, 12345, f.core);
  assert.deepEqual(f.state.writes, []);
  f.state.pr.head.sha = env.FRAMERIZE_PR_HEAD_SHA;
  f.state.jobs[0].conclusion = 'skipped';
  await assert.rejects(runPreview(f.github, repo, 12345, f.core));
  assert.deepEqual(f.state.writes, []);
});

test('a newer CI attempt during staging cannot publish the previous result', async t => {
  const f = fixture(t);
  f.state.onDownload = () => { f.state.run.run_attempt = 3; };
  await runPreview(f.github, repo, 12345, f.core);
  assert.deepEqual(f.state.writes, []);
  assert.match(f.state.logs.at(-1), /newer CI attempt/);
});

test('closing a PR during upload removes its files without advertising them', async t => {
  const f = fixture(t);
  f.state.onUpload = () => { f.state.pr.state = 'closed'; };
  await runPreview(f.github, repo, 12345, f.core);
  assert.equal(f.state.assets.length, 0);
  assert.equal(f.state.comments.length, 0);
});

test('an existing asset with different bytes is never overwritten', async t => {
  const f = fixture(t);
  f.state.assets.push({ id: 4, name: 'package.tgz', size: 3, digest: 'sha256:wrong' });
  await assert.rejects(upload(f.github, repo, { id: 10 }, new Map([['package.tgz', Buffer.from('abc')]])));
  assert.deepEqual(f.state.writes, []);
});

test('only the workflow bot can own the preview marker comment', async t => {
  const f = fixture(t);
  f.state.comments.push({ id: 9, body: marker + '\nUser text', user: { login: 'contributor' } });
  await updateComment(f.github, repo, 23, marker + '\nBuild');
  assert.equal(f.state.comments[0].body, marker + '\nUser text');
  assert.equal(f.state.comments.length, 2);
  assert.deepEqual(f.state.writes, [['comment-create']]);
});

test('a malformed ZIP fails without publication and removes its staging directory', async () => {
  const f = mock(Buffer.from('invalid ZIP'), []);
  const before = readdirSync(tmpdir()).filter(name => name.startsWith('framerize-publish-'));
  await assert.rejects(runPreview(f.github, repo, 12345, f.core));
  assert.deepEqual(f.state.writes, []);
  assert.deepEqual(readdirSync(tmpdir()).filter(name => name.startsWith('framerize-publish-')), before);
});
