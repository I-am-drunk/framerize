// Trust split and one-comment updates adapt T3 desktop-macos-preview-publish.yml (MIT).
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { validatePreview, commentBody, marker, hash } = require('./pr-preview-manifest.cjs');
const tag = 'framerize-pr-builds';

async function releaseFor(github, repo, create) {
  try {
    return (await github.rest.repos.getReleaseByTag({ ...repo, tag })).data;
  } catch (error) {
    if (error.status !== 404) throw error;
    if (!create) return null;
    try {
      return (await github.rest.repos.createRelease({ ...repo, tag_name: tag,
        name: 'Pull request previews', prerelease: true,
        body: 'Temporary PR test packages. Each build records its tested commit and checksums.' })).data;
    } catch (conflict) {
      if (conflict.status !== 422) throw conflict;
      return (await github.rest.repos.getReleaseByTag({ ...repo, tag })).data;
    }
  }
}

function belongsToPr(name, number) {
  assert.ok(Number.isSafeInteger(number) && number > 0);
  return new RegExp(`^(?:framerize(?:-remote)?-\\d+\\.\\d+\\.\\d+-pr\\.${number}\\.\\d+\\.g[0-9a-f]{12}\\.tgz|pr-${number}-\\d+-\\d+-(?:preview\\.json|producer\\.json|SHA256SUMS))$`).test(name);
}

async function removeOld(github, repo, release, number, keep = new Set()) {
  const assets = await github.paginate(github.rest.repos.listReleaseAssets, { ...repo, release_id: release.id });
  for (const asset of assets) {
    if (belongsToPr(asset.name, number) && !keep.has(asset.name)) {
      await github.rest.repos.deleteReleaseAsset({ ...repo, asset_id: asset.id });
    }
  }
}

async function updateComment(github, repo, number, body, create = true) {
  const comments = await github.paginate(github.rest.issues.listComments, { ...repo, issue_number: number });
  const previous = comments.find(comment => comment.user?.login === 'github-actions[bot]'
    && comment.body?.startsWith(marker));
  if (previous) {
    await github.rest.issues.updateComment({ ...repo, comment_id: previous.id, body });
  } else if (create) await github.rest.issues.createComment({ ...repo, issue_number: number, body });
}

async function upload(github, repo, release, files) {
  const assets = await github.paginate(github.rest.repos.listReleaseAssets, { ...repo, release_id: release.id });
  const urls = new Map();
  for (const [name, bytes] of files) {
    let asset = assets.find(entry => entry.name === name);
    if (asset) {
      assert.equal(asset.size, bytes.length, 'Existing preview has different bytes');
      if (asset.digest) assert.equal(asset.digest, `sha256:${hash(bytes)}`);
      else {
        const prior = await github.rest.repos.getReleaseAsset({ ...repo, asset_id: asset.id,
          headers: { accept: 'application/octet-stream' } });
        assert.equal(hash(Buffer.from(prior.data)), hash(bytes), 'Existing preview has different bytes');
      }
    } else {
      asset = (await github.rest.repos.uploadReleaseAsset({ ...repo, release_id: release.id,
        name, data: bytes, headers: { 'content-type': 'application/octet-stream' } })).data;
    }
    urls.set(name, asset.browser_download_url);
  }
  return urls;
}

async function runPreview(github, repo, runId, core) {
  const run = (await github.rest.actions.getWorkflowRun({ ...repo, run_id: runId })).data;
  if (run.event !== 'pull_request' || run.status !== 'completed' || run.conclusion !== 'success'
      || run.path !== '.github/workflows/ci.yml') return core.info('No completed PR CI run to publish.');
  assert.equal(run.repository.full_name.toLowerCase(), `${repo.owner}/${repo.repo}`.toLowerCase());
  const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRun,
    { ...repo, run_id: runId, filter: 'latest' });
  const checks = jobs.filter(job => job.name === 'Check');
  assert.equal(checks.length, 1, 'Expected the single required Check job');
  assert.equal(checks[0].conclusion, 'success');
  const linked = run.pull_requests.length ? run.pull_requests
    : await github.paginate(github.rest.repos.listPullRequestsAssociatedWithCommit,
      { ...repo, commit_sha: run.head_sha });
  const candidates = linked.filter(pr => pr.head.sha === run.head_sha
    && pr.base.repo.id === run.repository.id);
  if (candidates.length !== 1) return core.info('No unique PR is associated with this run.');
  const pr = (await github.rest.pulls.get({ ...repo, pull_number: candidates[0].number })).data;
  if (pr.state !== 'open' || pr.head.sha !== run.head_sha || !pr.merge_commit_sha) {
    return core.info('The PR moved or closed; this preview is obsolete.');
  }
  const artifacts = await github.paginate(github.rest.actions.listWorkflowRunArtifacts,
    { ...repo, run_id: runId });
  const matches = artifacts.filter(entry => entry.name === `pr-preview-${runId}` && !entry.expired);
  if (!matches.length) return core.info('This CI run did not produce a PR preview.');
  assert.equal(matches.length, 1);
  assert.ok(matches[0].size_in_bytes > 0 && matches[0].size_in_bytes <= 36 * 1024 * 1024);
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'framerize-publish-'));
  try {
    const archive = await github.rest.actions.downloadArtifact({ ...repo,
      artifact_id: matches[0].id, archive_format: 'zip' });
    const bytes = Buffer.from(archive.data);
    assert.ok(bytes.length <= 36 * 1024 * 1024);
    const zip = path.join(folder, 'input.zip');
    fs.writeFileSync(zip, bytes, { flag: 'wx', mode: 0o600 });
    const staged = path.join(folder, 'staged');
    execFileSync('python3', [path.join(__dirname, 'stage-pr-preview.py'), zip, staged],
      { timeout: 30_000, stdio: 'pipe' });
    const claim = JSON.parse(fs.readFileSync(path.join(staged, 'preview.json'), 'utf8'));
    if (claim.buildSha !== pr.merge_commit_sha) return core.info('The tested merge is obsolete.');
    const commit = (await github.rest.repos.getCommit({ ...repo, ref: pr.merge_commit_sha })).data;
    const candidate = validatePreview(staged, run, pr, commit);
    const current = (await github.rest.actions.getWorkflowRun({ ...repo, run_id: runId })).data;
    if (current.run_attempt !== run.run_attempt || current.conclusion !== 'success'
        || current.status !== 'completed') return core.info('A newer CI attempt is pending.');
    const release = await releaseFor(github, repo, true);
    const urls = await upload(github, repo, release, candidate.files);
    const fresh = (await github.rest.pulls.get({ ...repo, pull_number: pr.number })).data;
    if (fresh.state !== 'open') {
      await removeOld(github, repo, release, pr.number);
      return core.info('PR closed during upload; its preview files were removed.');
    }
    if (fresh.head.sha !== pr.head.sha
        || fresh.merge_commit_sha !== pr.merge_commit_sha) return core.info('PR changed during upload.');
    await updateComment(github, repo, pr.number,
      commentBody(candidate.manifest, urls, candidate.prefix));
    await removeOld(github, repo, release, pr.number, new Set(candidate.files.keys()));
  } finally {
    fs.rmSync(folder, { recursive: true, force: true });
  }
}

async function publish({ github, context, core }) {
  const repo = context.repo;
  if (context.eventName === 'pull_request_target') {
    assert.equal(context.payload.action, 'closed');
    const pr = context.payload.pull_request;
    const release = await releaseFor(github, repo, false);
    if (release) await removeOld(github, repo, release, pr.number);
    await updateComment(github, repo, pr.number,
      `${marker}\nThis PR is closed. Its temporary preview downloads were removed.`, false);
    return;
  }
  assert.equal(context.eventName, 'workflow_run');
  return runPreview(github, repo, context.payload.workflow_run.id, core);
}

module.exports = { publish, runPreview, belongsToPr, upload, updateComment };
