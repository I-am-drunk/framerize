const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { createHash } = require('node:crypto');
const { versionFor } = require('./preview-version.cjs');
const hash = bytes => createHash('sha256').update(bytes).digest('hex');

function read(root, name, limit) {
  const file = path.join(root, name);
  const info = fs.lstatSync(file);
  assert.ok(info.isFile() && info.size > 0 && info.size <= limit, 'Invalid preview input');
  const bytes = fs.readFileSync(file);
  assert.ok(bytes.length <= limit, 'Preview input exceeds its limit');
  return bytes;
}

function validatePreview(root, run, pr, commit) {
  const raw = read(root, 'preview.json', 2 * 1024 * 1024);
  const manifest = JSON.parse(raw.toString('utf8'));
  assert.equal(manifest.schema, 1);
  assert.equal(manifest.kind, 'pull-request');
  assert.equal(manifest.repository, run.repository.full_name);
  assert.equal(manifest.pr, String(pr.number));
  assert.equal(manifest.headSha, run.head_sha);
  assert.equal(manifest.headSha, pr.head.sha);
  assert.equal(manifest.buildSha, pr.merge_commit_sha, 'Stale preview merge commit');
  assert.equal(manifest.buildSha, commit.sha);
  assert.equal(manifest.tree, commit.commit.tree.sha);
  assert.equal(manifest.runId, String(run.id));
  assert.match(manifest.attempt, /^[1-9][0-9]{0,5}$/);
  assert.ok(Number(manifest.attempt) <= run.run_attempt, 'Future producer attempt');
  assert.equal(manifest.node, 'v22.23.4');
  assert.match(manifest.npm, /^\d+\.\d+\.\d+$/);
  assert.deepEqual(commit.parents.map(p => p.sha).slice(1), [pr.head.sha]);
  assert.deepEqual(manifest.packages.map(p => p.name), ['framerize-remote', 'framerize']);
  const files = new Map();
  for (const entry of manifest.packages) {
    assert.equal(entry.version, versionFor(entry.version, manifest));
    assert.equal(entry.tarball, `${entry.name}-${entry.version}.tgz`);
    const bytes = read(root, entry.tarball, 16 * 1024 * 1024);
    assert.equal(entry.bytes, bytes.length);
    assert.equal(entry.sha256, hash(bytes));
    files.set(entry.tarball, bytes);
  }
  const producer = read(root, 'producer.json', 2 * 1024 * 1024);
  assert.equal(manifest.producerManifestSha256, hash(producer));
  const original = JSON.parse(producer.toString('utf8'));
  assert.deepEqual(original.packages, manifest.packages);
  for (const [key, value] of Object.entries({ repository: manifest.repository,
    sha: manifest.buildSha, tree: manifest.tree, runId: manifest.runId,
    runAttempt: manifest.attempt, event: 'pull_request' })) {
    assert.equal(original.source[key], value);
  }
  assert.equal(original.node, manifest.node);
  assert.equal(original.npm, manifest.npm);
  assert.deepEqual(fs.readdirSync(root).sort(), [...files.keys(), 'preview.json', 'producer.json'].sort());
  const prefix = `pr-${pr.number}-${run.id}-${manifest.attempt}`;
  files.set(`${prefix}-preview.json`, raw);
  files.set(`${prefix}-producer.json`, producer);
  const sums = [...files].map(([name, bytes]) => `${hash(bytes)}  ${name}`).join('\n') + '\n';
  files.set(`${prefix}-SHA256SUMS`, Buffer.from(sums));
  return { manifest, files, prefix };
}

const marker = '<!-- framerize-pr-build -->';
function commentBody(manifest, urls, prefix) {
  const server = manifest.packages.find(p => p.name === 'framerize');
  const remote = manifest.packages.find(p => p.name === 'framerize-remote');
  return [marker, '### Framerize PR build', '',
    `PR head: \`${manifest.headSha}\``,
    `Tested merge commit: \`${manifest.buildSha}\``,
    `Package CI passed in [run ${manifest.runId}](https://github.com/${manifest.repository}/actions/runs/${manifest.runId}).`,
    '', 'Install into two temporary prefixes. These commands do not change your global installation.',
    '```sh', 'preview="$(mktemp -d "${TMPDIR:-/tmp}/framerize-preview.XXXXXX")"',
    `npm install --global --ignore-scripts --prefix "$preview/server" '${urls.get(server.tarball)}'`,
    `npm install --global --ignore-scripts --prefix "$preview/remote" '${urls.get(remote.tarball)}'`,
    '"$preview/server/bin/framerize" --version',
    '"$preview/remote/bin/framerize" --version', '```', '',
    `[Checksums](${urls.get(`${prefix}-SHA256SUMS`)}) · ` +
      `[Build record](${urls.get(`${prefix}-preview.json`)})`,
    '', `[Run with separate state and ports, or use PowerShell](https://github.com/${manifest.repository}/blob/main/docs/pr-previews.md).`,
    'Use foreground serve for previews. Do not use a preview to manage an installed service.',
    'These are PR test packages, not npm releases or live-provider qualification.',
  ].join('\n');
}

module.exports = { validatePreview, commentBody, marker, hash };
