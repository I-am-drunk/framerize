// The inert-docs assertion adapts T3 Code scripts/check-changed.test.ts at 5a906583 (MIT).
// Dependency selection, full fallback and Check results are project-specific tests.
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { readFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import test from 'node:test';

const { decide, readWorkspace } = createRequire(import.meta.url)('../.github/scripts/ci-changes.cjs');
const read = file => readFileSync(new URL('../' + file, import.meta.url), 'utf8');
const packages = readWorkspace();
const gates = files => decide(files, packages, false).outputs;
const all = { source: true, packages: true, server: true, performance: true };

test('root guides and agent skills are inert, package guides are not', () => {
  for (const file of ['README.md', 'AGENTS.md', 'docs/wip-status.md', '.agents/skills/test-framerize/SKILL.md']) {
    assert.deepEqual(decide([file], packages, false).affected, [], file);
    assert.equal(gates([file]).performance, false, file);
  }
  assert.deepEqual(gates(['framerize-remote/README.md']),
    { source: true, packages: true, server: false, performance: false });
  assert.deepEqual(gates(['framerize-server/README.md']), { ...all, performance: false });
});

test('a core change runs both packages that bundle it', () => {
  assert.deepEqual(decide(['framerize-core/src/instance.ts'], packages, false).affected,
    ['@framerize/core', 'framerize', 'framerize-remote']);
  assert.deepEqual(gates(['framerize-core/src/instance.ts']), all);
});

test('root tooling, fixtures, manifests and workflows run every job', () => {
  for (const file of ['package.json', 'package-lock.json', 'tsconfig.tools.json', '.gitattributes',
    'scripts/fixtures/installed-server.ts', 'scripts/fixtures/transport-scenario.ts',
    'test/installed-remote-flow.test.ts', 'test/installed-server-flow.test.ts',
    'test/installed-custom-routes.test.ts', 'test/custom-routes.test.mjs',
    'scripts/check-inference.ts', 'scripts/validation/inference.ts', 'test/inference-check.test.ts',
    'scripts/check-native-server.ts', 'scripts/native-account-smoke.ts',
    '.github/workflows/server-ci.yml', '.github/scripts/ci-changes.cjs', 'framerize-remote/package.json']) {
    assert.deepEqual(gates([file]), all, file);
  }
});

test('performance skips unrelated CLI edits without weakening package checks', () => {
  for (const file of ['framerize-server/src/cli.ts', 'framerize-server/src/setup.ts',
    'framerize-remote/src/clientCli.ts', 'framerize-remote/src/serviceCli.ts',
    'framerize-remote/src/cliArgs.ts', 'framerize-remote/src/cmdSetup.ts']) {
    const selected = gates([file]);
    assert.equal(selected.performance, false, file);
    assert.equal(selected.source, true, file);
    assert.equal(selected.packages, true, file);
  }
  for (const file of ['test/request-performance.test.mjs', 'test/custom-request-cases.mjs',
    'test/custom-routes-fixture.mjs', '.github/workflows/request-performance.yml',
    'framerize-server/src/framer-backend.ts', 'framerize-remote/src/remoteClient.ts',
    'framerize-server/src/new-unknown-handler.ts', 'unknown-file']) {
    assert.equal(gates([file]).performance, true, file);
  }
  assert.equal(gates(['docs/wip-status.md', 'framerize-core/src/http-io.ts']).performance, true);
});

test('unresolved diffs and full runs select everything', () => {
  assert.deepEqual(decide(null, packages, false).outputs, all);
  assert.deepEqual(decide(['README.md'], packages, true).outputs, all);
  assert.deepEqual(gates([]), { source: false, packages: false, server: false, performance: false });
});

test('performance never skips an imported request-path dependency', async () => {
  const result = await build({
    absWorkingDir: fileURLToPath(new URL('../', import.meta.url)),
    entryPoints: ['framerize-core/src/instance.ts', 'framerize-server/src/server.ts',
      'framerize-server/src/framer-backend.ts', 'framerize-remote/src/remoteBackend.ts'],
    external: ['../dist/core.js'], outdir: 'unused-performance-dependency-check',
    bundle: true, write: false, metafile: true, platform: 'node', format: 'esm',
  });
  for (const file of Object.keys(result.metafile.inputs)) {
    assert.equal(gates([file]).performance, true, file);
  }
});

test('performance is wired through detection, its workflow and the required Check', () => {
  const ci = read('.github/workflows/ci.yml');
  assert.match(read('.github/actions/ci-changes/action.yml'),
    /performance:\n    value: \$\{\{ steps\.detect\.outputs\.performance \}\}/);
  assert.match(ci, /performance: \$\{\{ steps\.detect\.outputs\.performance \}\}/);
  assert.match(ci, /  performance:\n    needs: changes\n    if: needs\.changes\.outputs\.performance != 'false'\n    uses: \.\/\.github\/workflows\/request-performance\.yml/);
  assert.match(ci.slice(ci.indexOf('  check:')), /needs: \[[^\]]*performance\]/);
  const workflow = read('.github/workflows/request-performance.yml');
  assert.match(workflow, /permissions:\n  contents: read/);
  assert.match(workflow, /node: \['22\.23\.4', '24\.13\.0'\]/);
  assert.match(workflow, /run: npm run build/);
  assert.match(workflow, /run: npm run test:performance/);
  assert.match(JSON.parse(read('package.json')).scripts['test:performance'],
    /test\/request-performance\.test\.mjs/);
  assert.ok(workflow.indexOf('run: npm run build') < workflow.indexOf('run: npm run test:performance'));
  assert.match(workflow, /FRAMERIZE_PERFORMANCE_REPORT: \$\{\{ runner.temp \}\}\/request-performance.json/);
  assert.match(workflow, /path: \$\{\{ runner.temp \}\}\/request-performance.json/);
});

test('the actual Check expression rejects failed, cancelled and unexpected skipped jobs', () => {
  const expression = read('.github/workflows/ci.yml')
    .match(/jq -e --arg skip "\$SKIP_SERVER" '([\s\S]*?)' > \/dev\/null/)?.[1];
  assert.ok(expression, 'Check must evaluate the recorded job results');
  const jobs = ['changes', 'source', 'packages', 'server', 'performance'];
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
  docs.changes.outputs = { source: 'false', packages: 'false', server: 'false', performance: 'false' };
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
  const performance = successful();
  performance.performance.result = 'skipped';
  performance.changes.outputs = { performance: 'false' };
  check(performance, true);
  performance.changes.outputs = { source: 'false' };
  check(performance, false);
  performance.changes.outputs.performance = 'false';
  performance.performance.result = 'failure';
  check(performance, false);
  performance.performance.result = 'cancelled';
  check(performance, false);
});
