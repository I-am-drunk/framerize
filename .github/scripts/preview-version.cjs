// PR versioning follows T3's resolve-nightly-release.ts at fbe5df2d (MIT).
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const packages = [['framerize-remote', 'framerize-remote'], ['framerize-server', 'framerize']];

function context(env) {
  if (env.GITHUB_EVENT_NAME !== 'pull_request') return null;
  for (const name of ['FRAMERIZE_PR_NUMBER', 'GITHUB_RUN_ID', 'GITHUB_RUN_ATTEMPT']) {
    assert.match(env[name] ?? '', /^[1-9][0-9]{0,19}$/, `Invalid ${name}`);
  }
  for (const name of ['FRAMERIZE_PR_HEAD_SHA', 'GITHUB_SHA']) {
    assert.match(env[name] ?? '', /^[0-9a-f]{40}$/, `Invalid ${name}`);
  }
  assert.match(env.GITHUB_REPOSITORY ?? '', /^[\w.-]+\/[\w.-]+$/);
  return { repository: env.GITHUB_REPOSITORY, pr: env.FRAMERIZE_PR_NUMBER,
    headSha: env.FRAMERIZE_PR_HEAD_SHA, buildSha: env.GITHUB_SHA,
    runId: env.GITHUB_RUN_ID, attempt: env.GITHUB_RUN_ATTEMPT };
}

function versionFor(base, info) {
  const match = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[\w.-]+)?(?:\+[\w.-]+)?$/.exec(base);
  assert.ok(match, 'Invalid base package version');
  const core = match.slice(1, 4).join('.');
  return `${core}-pr.${info.pr}.${info.runId}.g${info.buildSha.slice(0, 12)}`;
}

function stamp(root, env = process.env) {
  const info = context(env);
  if (!info) return null;
  const changes = packages.map(([directory, name]) => {
    const file = path.join(root, directory, 'package.json');
    const manifest = JSON.parse(fs.readFileSync(file, 'utf8'));
    assert.equal(manifest.name, name, 'Unexpected package name');
    const version = versionFor(manifest.version, info);
    return { file, name, version, manifest: { ...manifest, version } };
  });
  for (const entry of changes) {
    fs.writeFileSync(entry.file, JSON.stringify(entry.manifest, null, 2) + '\n');
  }
  return changes.map(({ name, version }) => ({ name, version }));
}

function exportPreview(input, output, env = process.env) {
  const info = context(env);
  assert.ok(info, 'Preview export requires a pull request');
  const raw = fs.readFileSync(path.join(input, 'manifest.json'));
  const producer = JSON.parse(raw.toString('utf8'));
  assert.equal(producer.source.sha, info.buildSha);
  assert.equal(producer.source.repository, info.repository);
  assert.equal(producer.source.runId, info.runId);
  assert.equal(producer.source.runAttempt, info.attempt);
  assert.equal(producer.source.event, 'pull_request');
  assert.match(producer.source.tree, /^[0-9a-f]{40}$/);
  assert.deepEqual(producer.packages.map(p => p.name), packages.map(p => p[1]));
  const files = producer.packages.map(entry => {
    assert.equal(entry.version, versionFor(entry.version, info));
    assert.equal(entry.tarball, `${entry.name}-${entry.version}.tgz`);
    const file = path.join(input, entry.tarball);
    const stat = fs.lstatSync(file);
    assert.ok(stat.isFile() && stat.size <= 16 * 1024 * 1024);
    const bytes = fs.readFileSync(file);
    assert.equal(bytes.length, entry.bytes);
    assert.equal(digest(bytes), entry.sha256);
    return [entry.tarball, bytes];
  });
  const manifest = { schema: 1, kind: 'pull-request', ...info,
    tree: producer.source.tree, node: producer.node, npm: producer.npm,
    producerManifestSha256: digest(raw), packages: producer.packages };
  fs.mkdirSync(output);
  for (const [name, bytes] of files) fs.writeFileSync(path.join(output, name), bytes);
  fs.writeFileSync(path.join(output, 'producer.json'), raw);
  fs.writeFileSync(path.join(output, 'preview.json'), JSON.stringify(manifest, null, 2) + '\n');
  return manifest;
}

module.exports = { context, versionFor, stamp, exportPreview };
if (require.main === module) {
  try {
    if (process.argv[2] === 'stamp') stamp(process.cwd());
    else if (process.argv[2] === 'export' && process.argv.length === 5) {
      exportPreview(process.argv[3], process.argv[4]);
    } else throw new Error('Expected stamp or export INPUT OUTPUT');
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
