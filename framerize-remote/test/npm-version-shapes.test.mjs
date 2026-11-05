import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';
import { posix } from 'node:path';

const npmPath = posix.join('/synthetic/bin', 'npm');

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/latest.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { checkVersion } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].text).toString('base64')}`);

for (const [response, expected] of [
  [['1.0.0 invalid'], { status: 'unknown', current: '1.0.0', latest: null }],
  [['1.0.0', '2.0.0'], { status: 'unknown', current: '1.0.0', latest: null }],
  [['1.0.0-beta.1+build.2'], { status: 'current', current: '1.0.0', latest: '1.0.0-beta.1+build.2' }],
]) {
  test(`combined npm response validation: ${JSON.stringify(response)}`, async () => {
    const calls = [];
    const forbidden = async () => assert.fail('unexpected file, network or update operation');
    const result = await checkVersion('claude', '/synthetic/claude', {
      kind: 'manual', reason: 'synthetic fixture', owner: { kind: 'npm', prefix: '/synthetic/npm' },
    }, {
      platform: 'linux', home: '/synthetic/home', env: { PATH: '/synthetic/bin' },
      probe: path => path === npmPath, read: forbidden, fetch: forbidden,
      runner: { run: forbidden, capture: async command => {
        calls.push([command.command, command.args]);
        return command.command === '/synthetic/claude' ? 'Claude Code 1.0.0' : JSON.stringify(response);
      } },
    });
    assert.deepEqual(calls, [
      ['/synthetic/claude', ['--version']],
      [npmPath, ['view', '--global', '--prefix', '/synthetic/npm', '--json', '@anthropic-ai/claude-code@latest', 'version']],
    ]);
    assert.deepEqual(result, expected);
  });
}
