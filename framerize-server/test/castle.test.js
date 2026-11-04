import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const serverRoot = fileURLToPath(new URL('..', import.meta.url));
const repoRoot = fileURLToPath(new URL('../..', import.meta.url));

const LEGACY_REALM_PATHS = [
  'scripts/offline_worker/realm_globals.mjs',
  'scripts/offline_worker/realm_window.mjs',
];

test('legacy offline realm modules stay out of version control', () => {
  const gitignore = readFileSync(path.join(repoRoot, '.gitignore'), 'utf8');
  for (const relative of LEGACY_REALM_PATHS) {
    assert.match(gitignore, new RegExp(`^/${relative.replaceAll('.', '\\.')}$`, 'm'));
    assert.equal(existsSync(path.join(repoRoot, relative)), false);
    const tracked = spawnSync('git', ['ls-files', '--', relative], {
      cwd: repoRoot, encoding: 'utf8',
    });
    assert.equal(tracked.status, 0, tracked.stderr);
    assert.equal(tracked.stdout.trim(), '');
  }
});

test('Castle ships only from the server package source inventory', () => {
  const check = readFileSync(path.join(serverRoot, 'scripts/check.js'), 'utf8');
  assert.match(check, /'castle\.ts'/);
  assert.doesNotMatch(check, /offline_worker|realm_globals|realm_window/);
  const sources = readFileSync(path.join(serverRoot, 'test/check.test.js'), 'utf8');
  assert.match(sources, /'castle\.ts'/);
  assert.doesNotMatch(sources, /offline_worker|realm_globals|realm_window/);
});

test('Castle mint releases realm timers so a child process can exit', { timeout: 20_000 }, () => {
  const script = `
    import assert from 'node:assert/strict';
    import { createCastleMinter } from './dist/castle.js';
    const minter = createCastleMinter({ budgetMs: 8000, timeoutMs: 9000 });
    const { token } = await minter.mint({});
    assert.equal(typeof token, 'string');
    assert.ok(token.length >= 64);
    await minter.close();
  `;
  const result = spawnSync(process.execPath, ['--input-type=module', '-e', script], {
    cwd: serverRoot, encoding: 'utf8', timeout: 15_000,
  });
  assert.equal(result.error, undefined, result.error?.message);
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
});
