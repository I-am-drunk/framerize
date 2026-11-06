import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdtempSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { parseHomebrewCaskVersion } from './versions.ts'; export { offerClients } from './offer.ts';",
  resolveDir: fileURLToPath(new URL('../src/compatibility/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false });
const bytes = compiled.outputFiles[0].contents;
const bundleHash = createHash('sha256').update(bytes).digest('hex');
const { parseHomebrewCaskVersion, offerClients } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

function fixture(context, version) {
  const home = mkdtempSync(join(tmpdir(), 'frm-homebrew-metadata-'));
  context.after(() => { rmSync(home, { recursive: true, force: true }); assert.equal(existsSync(home), false); });
  const executable = '/synthetic-homebrew/bin/claude';
  const brew = '/synthetic-homebrew/bin/brew';
  const captures = [], unexpected = [], output = [];
  const metadata = JSON.stringify({ casks: [{ version }] });
  const deps = {
    platform: 'darwin', env: { HOME: '/synthetic-homebrew', PATH: '/synthetic-homebrew/bin' },
    home: '/synthetic-homebrew',
    stateDir: join(home, 'state'), target: { origin: 'http://127.0.0.1:8788' },
    probe: path => path === executable || path === brew,
    realPath: () => '/synthetic-homebrew/Caskroom/claude-code/2.1.1/claude',
    io: { stdout: text => output.push(text), stderr: text => unexpected.push(['stderr', text]) },
    fetch: async (...args) => { unexpected.push(['fetch', args]); return null; },
    read: async (...args) => { unexpected.push(['read', args]); return null; },
    runner: {
      capture: async command => {
        captures.push([command.command, command.args]);
        if (command.command === executable) return '2.1.1 (synthetic client)\n';
        if (command.command === brew) return metadata;
        unexpected.push(['capture', command]);
        return null;
      },
      run: async command => { unexpected.push(['run', command]); return 1; },
    },
  };
  return { home, executable, brew, metadata, deps, captures, unexpected, output };
}

const cases = [
  ['0.0.0oops', null, 'unknown'],
  ['2.1.1-rc..1', null, 'unknown'],
  ['02.1.0', null, 'unknown'],
  ['2.1.0', '2.1.0', 'current'],
  ['2.1.2', '2.1.2', 'behind'],
  ['2.1.2,45', '2.1.2', 'behind'],
  ['2.1.2-rc.1+build.2', '2.1.2-rc.1+build.2', 'behind'],
  ['latest', null, 'unknown'],
];

for (const [version, expectedVersion, expectedStatus] of cases) {
  test(`Homebrew metadata ${version} gives ${expectedStatus}`, { timeout: 2000 }, async context => {
    const state = fixture(context, version);
    const parsed = parseHomebrewCaskVersion(state.metadata);
    const results = await offerClients(state.deps, ['claude']);
    const output = state.output.join('');
    context.diagnostic(JSON.stringify({ bundleHash, parsed, results, output, captures: state.captures }));
    assert.deepEqual(state.captures, [[state.executable, ['--version']],
      [state.brew, ['info', '--json=v2', '--cask', 'claude-code']]]);
    assert.deepEqual(state.unexpected, []);
    assert.deepEqual(readdirSync(state.home), []);
    assert.deepEqual({ parsed, results }, { parsed: expectedVersion,
      results: [{ client: 'claude', state: 'present', version: expectedStatus }] });
    if (expectedStatus === 'unknown') {
      assert.match(output, /the latest version could not be checked\./);
      assert.doesNotMatch(output, /is the latest\.|To update Claude Code:/);
    } else if (expectedStatus === 'behind') {
      assert.match(output, /To update Claude Code: brew upgrade --cask claude-code/);
    } else {
      assert.match(output, /Version 2\.1\.1 is the latest\./);
    }
  });
}
