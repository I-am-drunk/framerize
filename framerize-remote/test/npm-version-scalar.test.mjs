import test from 'node:test';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const source = name => fileURLToPath(new URL(`../src/compatibility/${name}.ts`, import.meta.url));
const compiled = await build({ stdin: { contents:
  `export { checkVersion } from ${JSON.stringify(source('latest'))};\nexport { versionLine } from ${JSON.stringify(source('versions'))};`,
  resolveDir: fileURLToPath(new URL('.', import.meta.url)) },
bundle: true, platform: 'node', format: 'esm', write: false });
const { checkVersion, versionLine } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function lookup(latest) {
  const captures = [], controller = new AbortController();
  const executable = '/synthetic/bin/claude', prefix = '/synthetic/npm-prefix';
  const env = { PATH: '/synthetic/bin' };
  const deps = { platform: 'linux', env, home: '/synthetic/home', signal: controller.signal,
    probe: path => path === '/synthetic/bin/npm',
    read: async () => assert.fail('npm version lookup must not read settings'),
    fetch: async () => assert.fail('npm version lookup must not fetch a native release'),
    runner: {
      run: async () => assert.fail('version lookup must not run an updater'),
      capture: async (command, receivedEnv, signal) => {
        assert.equal(receivedEnv, env);
        assert.equal(signal, controller.signal);
        captures.push({ command: command.command, args: command.args });
        return command.command === executable ? '2.1.0 (synthetic client)\n' : JSON.stringify(latest);
      },
    },
  };
  const result = await checkVersion('claude', executable, {
    kind: 'command', owner: { kind: 'npm', prefix },
    command: { command: 'npm', args: [], display: 'synthetic npm updater' },
  }, deps);
  assert.deepEqual(captures, [
    { command: executable, args: ['--version'] },
    { command: '/synthetic/bin/npm',
      args: ['view', '--global', '--prefix', prefix, '--json', '@anthropic-ai/claude-code@latest', 'version'] },
  ]);
  return result;
}

for (const latest of ['! invalid metadata', 'not-a-version', '2.1.0 invalid']) {
  test(`malformed npm scalar ${JSON.stringify(latest)} reports unknown`, async context => {
    const result = await lookup(latest);
    const line = versionLine(result);
    context.diagnostic(JSON.stringify({ latest, result, line }));
    assert.deepEqual(result, { current: '2.1.0', latest: null, status: 'unknown' });
    assert.equal(line, 'Version 2.1.0; the latest version could not be checked.');
  });
}

for (const [latest, status] of [
  ['2.1.0', 'current'], ['2.1.1', 'behind'],
  ['2.1.0-beta.1', 'current'], ['2.1.0+build.5', 'current'],
]) {
  test(`valid npm scalar ${latest} keeps its version status`, async () => {
    assert.deepEqual(await lookup(latest), { current: '2.1.0', latest, status });
  });
}
