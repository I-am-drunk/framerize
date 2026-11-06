import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
const ci = readFileSync(new URL('../.github/workflows/ci.yml', import.meta.url), 'utf8');
test('every PR requires packages even when source checks are skipped', () => {
  const expression = ci.match(/^      packages: \$\{\{ (.+) \}\}$/m)[1];
  const output = new Function('github', 'steps', `return (${expression});`);
  const steps = { detect: { outputs: { packages: 'false' } } };
  assert.equal(output({ event_name: 'pull_request' }, steps), 'true');
  assert.equal(output({ event_name: 'push' }, steps), 'false');
  const job = ci.match(/^  packages:\n([\s\S]*?)(?=^  \w+:)/m)[1];
  const condition = job.match(/if: \$\{\{ (.+) \}\}/)[1];
  const enabled = new Function('needs', 'cancelled', `const always = () => true; return (${condition});`);
  const needs = { changes: { result: 'success', outputs: { packages: 'true' } }, source: { result: 'skipped' } };
  assert.equal(enabled(needs, () => false), true);
  assert.equal(enabled(needs, () => true), false);
  needs.source.result = 'failure';
  assert.equal(enabled(needs, () => false), false);
  assert.doesNotMatch(job, /secrets: inherit/);
});

test('the publisher runs trusted default-branch code and separates PR queues', () => {
  const source = readFileSync(new URL('../.github/workflows/pr-previews.yml', import.meta.url), 'utf8');
  assert.match(source, /ref: \$\{\{ github.event.repository.default_branch \}\}/);
  assert.match(source, /persist-credentials: false/);
  assert.match(source, /workflows: \[ci\]/);
  assert.match(source, /pull_request_target:\n    types: \[closed\]/);
  assert.match(source, /group: pr-preview-\$\{\{ needs.identify.outputs.pr \}\}/);
  assert.match(source, /needs: identify/);
  assert.doesNotMatch(source, /ref:.*(?:head_sha|head.sha)|npm (?:install|ci)|tooling/);
  assert.match(source, /require\('\.\/\.github\/scripts\/publish-pr-preview.cjs'\)/);
});

test('fork completions and close events resolve the same PR queue without global serialization', async () => {
  const source = readFileSync(new URL('../.github/workflows/pr-previews.yml', import.meta.url), 'utf8');
  const script = source.match(/          script: \|\n([\s\S]*?)\n  publish:/)[1]
    .split('\n').map(line => line.slice(12)).join('\n');
  const resolve = new (Object.getPrototypeOf(async () => {}).constructor)('github', 'context', script);
  const pr = { number: 23, head: { sha: 'a' }, base: { repo: { id: 1 } } };
  const github = { rest: { repos: {} }, paginate: async () => [pr] };
  const payload = { repository: { id: 1 }, workflow_run: { head_sha: 'a', pull_requests: [] } };
  assert.equal(await resolve(github, { repo: {}, payload }), 23);
  assert.equal(await resolve(github, { payload: { pull_request: pr } }), 23);
  payload.workflow_run.pull_requests = [pr];
  assert.equal(await resolve(github, { payload }), 23);
  payload.workflow_run.head_sha = 'other';
  assert.equal(await resolve(github, { payload }), '');
});
