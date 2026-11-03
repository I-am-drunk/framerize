import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const directory = fileURLToPath(new URL('..', import.meta.url));
const preload = new URL('./fixtures/smoke-failures.js', import.meta.url).href;
const cases = {
  'smoke.js': ['none', 'write', 'listen', 'invalid-config', 'fetch', 'spawn', 'early-exit', 'startup-timeout'],
  'framer-smoke.js': ['none', 'write', 'listen', 'invalid-config', 'fetch', 'app-listen'],
};

for (const [script, failures] of Object.entries(cases)) {
  for (const failure of failures) {
    test(`${script}: closes resources and removes its directory after ${failure}`, (context) => {
      const root = mkdtempSync(path.join(tmpdir(), 'nexus-smoke-cleanup-test-'));
      context.after(() => rmSync(root, { recursive: true, force: true }));
      const temporary = path.join(root, 'temporary');
      mkdirSync(temporary);
      writeFileSync(path.join(temporary, 'retained.txt'), 'pre-existing fixture');
      const report = path.join(root, 'report.json');
      const systemRoot = process.env.SystemRoot || 'C:\\Windows';
      const result = spawnSync(process.execPath, ['--import', preload, path.join('scripts', script)], {
        cwd: directory, timeout: 15000, encoding: 'utf8',
        env: {
          PATH: path.dirname(process.execPath), SystemRoot: systemRoot,
          HOME: root, USERPROFILE: root, TMPDIR: temporary, TMP: temporary, TEMP: temporary,
          NODE_DISABLE_COMPILE_CACHE: '1', NEXUS_SMOKE_FAILURE: failure, NEXUS_SMOKE_REPORT: report,
          FRAMER_ACCOUNTS: path.join(root, 'never-read.json'),
          FRAMER_UPSTREAM: 'http://127.0.0.1:1', FRAMER_HOST: '0.0.0.0', FRAMER_PORT: '1',
          FRAMER_SMOKE_JWT: 'saved-fixture-jwt', FRAMER_SMOKE_CASTLE: 'saved-fixture-castle',
        },
      });
      assert.equal(result.error, undefined, result.stderr);
      assert.equal(result.signal, null, result.stderr);
      assert.equal(result.status, failure === 'none' ? 0 : 1, result.stdout + result.stderr);
      assert.deepEqual(readdirSync(temporary), ['retained.txt']);
      assert.equal(readFileSync(path.join(temporary, 'retained.txt'), 'utf8'), 'pre-existing fixture');
      const evidence = JSON.parse(readFileSync(report, 'utf8'));
      assert.equal(path.dirname(evidence.directory), temporary);
      assert.equal(evidence.environmentRestored, true);
      if (failure === 'none') assert.match(result.stdout, /smoke passed/);
      if (failure === 'startup-timeout') assert.match(result.stderr, /readiness timeout/);
    });
  }
}
