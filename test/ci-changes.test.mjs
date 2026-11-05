// The inert-docs assertion adapts T3 Code scripts/check-changed.test.ts at 5a906583 (MIT).
// Dependency selection, full fallback and Check results are project-specific tests.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

const { decide, readWorkspace } = createRequire(import.meta.url)('../.github/scripts/ci-changes.cjs');
const read = file => readFileSync(new URL('../' + file, import.meta.url), 'utf8');
const packages = readWorkspace();
const gates = files => decide(files, packages, false).outputs;
const all = { source: true, packages: true, server: true };

test('root guides and agent skills are inert, package guides are not', () => {
  for (const file of ['README.md', 'AGENTS.md', 'docs/wip-status.md', '.agents/skills/test-framerize/SKILL.md']) {
    assert.deepEqual(decide([file], packages, false).affected, [], file);
  }
  assert.deepEqual(gates(['framerize-remote/README.md']), { source: true, packages: true, server: false });
  assert.deepEqual(gates(['framerize-server/README.md']), all);
});

test('a core change runs both packages that bundle it', () => {
  assert.deepEqual(decide(['framerize-core/src/instance.ts'], packages, false).affected,
    ['@framerize/core', 'framerize-remote', 'framerize-server']);
  assert.deepEqual(gates(['framerize-core/src/instance.ts']), all);
});

test('root tooling, fixtures, manifests and workflows run every job', () => {
  for (const file of ['package.json', 'package-lock.json', 'tsconfig.tools.json', '.gitattributes',
    'scripts/fixtures/installed-server.ts', 'scripts/fixtures/transport-scenario.ts',
    'test/installed-remote-flow.test.ts', 'scripts/native-account-smoke.ts',
    '.github/workflows/server-ci.yml', '.github/scripts/ci-changes.cjs', 'framerize-remote/package.json']) {
    assert.deepEqual(gates([file]), all, file);
  }
});

test('unresolved diffs and full runs select everything', () => {
  assert.deepEqual(decide(null, packages, false).outputs, all);
  assert.deepEqual(decide(['README.md'], packages, true).outputs, all);
  assert.deepEqual(gates([]), { source: false, packages: false, server: false });
});

test('the actual Check expression rejects failed, cancelled and unexpected skipped jobs', () => {
  const expression = read('.github/workflows/ci.yml')
    .match(/jq -e --arg skip "\$SKIP_SERVER" '([\s\S]*?)' > \/dev\/null/)?.[1];
  assert.ok(expression, 'Check must evaluate the recorded job results');
  const jobs = ['changes', 'source', 'packages', 'server'];
  const successful = () => Object.fromEntries(jobs.map(name => [name, { result: 'success' }]));
  function check(needs, expected, skipServer = false) {
    const result = spawnSync('jq', ['-e', '--arg', 'skip', String(skipServer), expression],
      { input: JSON.stringify(needs), encoding: 'utf8', timeout: 2000 });
    assert.equal(result.error, undefined);
    assert.equal(result.status, expected ? 0 : 1, result.stderr);
  }
  check(successful(), true);
  for (const job of jobs) for (const result of ['failure', 'cancelled', 'skipped']) {
    const needs = successful();
    needs[job].result = result;
    check(needs, false);
  }
  const docs = successful();
  docs.changes.outputs = { source: 'false', packages: 'false', server: 'false' };
  for (const job of jobs.slice(1)) docs[job].result = 'skipped';
  check(docs, true);
  docs.packages.result = 'failure';
  check(docs, false);
  const remote = successful();
  remote.changes.outputs = { server: 'false' };
  remote.server.result = 'skipped';
  check(remote, true);
  delete remote.changes.outputs;
  check(remote, false);
  check(remote, true, true);
});
