import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { join } from 'node:path';

const npmPath = join('/synthetic/bin', 'npm');

async function loadSource(name) {
  const compiled = await build({
    entryPoints: [fileURLToPath(new URL(`../src/compatibility/${name}.ts`, import.meta.url))],
    bundle: true, platform: 'node', format: 'esm', write: false,
  });
  return import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);
}
const { parseNpmViewVersion } = await loadSource('versions');
const { checkVersion } = await loadSource('latest');

test('npm version parsing rejects every multi-item result', () => {
  for (const versions of [['1.0.0', '2.0.0'], ['2.0.0', '1.0.0'],
    ['1.0.0', '1.0.0'], ['1.0.0', null], ['1.0.0', {}]]) {
    assert.equal(parseNpmViewVersion(JSON.stringify(versions)), null);
  }
});

test('npm version parsing retains scalar and single-item results', () => {
  assert.equal(parseNpmViewVersion('"1.2.3"'), '1.2.3');
  assert.equal(parseNpmViewVersion('[" 1.2.3-beta.1 "]'), '1.2.3-beta.1');
  for (const invalid of [null, '', '[]', '[null]', '[1]', '[{}]', '[""]', 'null', '{']) {
    assert.equal(parseNpmViewVersion(invalid), null);
  }
});

for (const versions of [['1.0.0', '2.0.0'], ['2.0.0', '1.0.0']]) {
  test(`ambiguous npm release ${versions.join('/')} produces unknown update status`, async () => {
    const captures = [];
    const unexpected = [];
    const deps = {
      platform: 'linux', home: '/synthetic/home', env: { PATH: '/synthetic/bin' },
      probe: candidate => candidate === npmPath,
      read: async () => { unexpected.push('read'); return null; },
      fetch: async () => { unexpected.push('fetch'); return null; },
      runner: {
        run: async () => { unexpected.push('run'); return 1; },
        capture: async command => {
          captures.push({ command: command.command, args: command.args });
          return command.args[0] === '--version' ? 'Claude Code 1.0.0' : JSON.stringify(versions);
        },
      },
    };
    const result = await checkVersion('claude', '/synthetic/bin/claude', {
      kind: 'manual', reason: 'synthetic fixture', owner: { kind: 'npm', prefix: '/synthetic/npm' },
    }, deps);
    assert.deepEqual(captures, [
      { command: '/synthetic/bin/claude', args: ['--version'] },
      { command: npmPath, args: ['view', '--global', '--prefix', '/synthetic/npm',
        '--json', '@anthropic-ai/claude-code@latest', 'version'] },
    ]);
    assert.deepEqual(unexpected, []);
    assert.deepEqual(result, { status: 'unknown', current: '1.0.0', latest: null });
  });
}
