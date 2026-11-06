import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, win32 } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/compatibility/index.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const bytes = compiled.outputFiles[0].contents;
const { checkVersion, planUpdate, versionLine } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
console.log(JSON.stringify({ node: process.version, platform: process.platform,
  sourceBundleSha256: createHash('sha256').update(bytes).digest('hex'),
  fixture: 'synthetic Windows command lookup; no real child or network' }));

const cases = [
  ['non-version', 'claude-code|not-a-version', null, 'unknown'],
  ['invalid suffix', 'claude-code|2.1.1oops', null, 'unknown'],
  ['empty prerelease identifier', 'claude-code|2.1.1-rc..1', null, 'unknown'],
  ['current release', 'claude-code|2.1.1', '2.1.1', 'current'],
  ['newer release', 'claude-code|2.1.2', '2.1.2', 'behind'],
  ['newer prerelease', 'claude-code|2.2.0-beta.1', '2.2.0-beta.1', 'behind'],
  ['missing version', 'claude-code|', null, 'unknown'],
  ['other package', 'not-claude-code|2.1.2', null, 'unknown'],
];

for (const [name, output, latest, status] of cases) {
  test(`Chocolatey metadata: ${name}`, { timeout: 3000 }, async context => {
    const home = await mkdtemp(join(tmpdir(), 'frm-choco-metadata-'));
    context.after(() => rm(home, { recursive: true, force: true }));
    const bin = 'C:\\synthetic\\chocolatey\\bin';
    const executable = win32.join(bin, 'claude.exe');
    const choco = win32.join(bin, 'choco.EXE');
    const env = { HOME: home, PATH: bin, PATHEXT: '.EXE' };
    const before = { ...env };
    const calls = [];
    const plan = planUpdate('claude', { resolvedPath: executable, realPath: executable, platform: 'win32', env });
    assert.deepEqual(plan, { kind: 'manual', reason: 'choco upgrade claude-code',
      owner: { kind: 'chocolatey' } });
    const result = await checkVersion('claude', executable, plan, {
      platform: 'win32', env, home,
      probe: candidate => candidate === choco,
      read: async () => assert.fail('No settings read is needed for Chocolatey'),
      fetch: async () => assert.fail('No release HTTP request is permitted'),
      runner: {
        run: async () => assert.fail('No installer, client or service may start'),
        capture: async (command, suppliedEnv) => {
          assert.equal(suppliedEnv, env);
          calls.push({ command: command.command, args: [...command.args] });
          if (command.command === executable) {
            assert.deepEqual(command.args, ['--version']);
            return 'Claude Code 2.1.1';
          }
          assert.equal(command.command, choco);
          assert.deepEqual(command.args, ['search', 'claude-code', '--exact', '--limit-output']);
          return `${output}\r\n`;
        },
      },
    });
    assert.deepEqual(calls, [
      { command: executable, args: ['--version'] },
      { command: choco, args: ['search', 'claude-code', '--exact', '--limit-output'] },
    ]);
    assert.deepEqual(env, before);
    assert.deepEqual(await readdir(home), []);
    await rm(home, { recursive: true, force: true });
    context.diagnostic(JSON.stringify({ observed: result, line: versionLine(result),
      privateStateRemoved: true, syntheticCaptures: calls.length }));
    assert.deepEqual(result, { current: '2.1.1', latest, status });
    if (status === 'unknown') assert.equal(versionLine(result),
      'Version 2.1.1; the latest version could not be checked.');
  });
}
