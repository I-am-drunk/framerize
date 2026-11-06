import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { parseInstalledVersion } from './versions.ts'; export { checkVersion } from './latest.ts'; export { runClient } from './command.ts';",
  resolveDir: fileURLToPath(new URL('../src/compatibility/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false });
const bytes = compiled.outputFiles[0].contents;
const bundleHash = createHash('sha256').update(bytes).digest('hex');
const { parseInstalledVersion, checkVersion, runClient } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

for (const version of ['2.1.0-rc..1', '2.1.0-rc.', '2.1.0-.rc',
  '2.1.0-01', '02.1.0', '2.01.0', '2.1.00']) {
  test(`malformed installed version ${version} remains unknown`, context => {
    context.diagnostic(`source bundle sha256 ${bundleHash}`);
    assert.equal(parseInstalledVersion(`${version} (synthetic client)\n`), null);
  });
}

test('valid zero and alphanumeric identifiers retain their installed versions', () => {
  for (const version of ['4.5.6', '4.5.6-0', '4.5.6-0a', '4.5.6-dev.0.z', '4.5.6-01x']) {
    assert.equal(parseInstalledVersion(`${version} (synthetic client)\n`), version);
  }
});

function fixture() {
  const executable = '/synthetic/home/.local/bin/claude';
  const captures = [], runs = [], fetches = [], reads = [], output = [], errors = [];
  const deps = {
    platform: 'linux', env: { PATH: '/synthetic/home/.local/bin' }, home: '/synthetic/home',
    stateDir: '/synthetic/state', target: { origin: 'http://127.0.0.1:8788' },
    probe: path => path === executable, realPath: path => path,
    io: { stdout: text => output.push(text), stderr: text => errors.push(text) },
    read: async path => { reads.push(path); return null; },
    fetch: async url => { fetches.push(url); return '2.1.0'; },
    runner: {
      capture: async command => { captures.push(command); return '2.1.1-rc..1 (synthetic client)\n'; },
      run: async command => { runs.push(command); return 0; },
    },
  };
  return { executable, deps, captures, runs, fetches, reads, output, errors };
}

test('malformed installed version does not qualify as current', { timeout: 2000 }, async context => {
  const state = fixture();
  const result = await checkVersion('claude', state.executable, { owner: { kind: 'native' } }, state.deps);
  context.diagnostic(JSON.stringify({ result, captures: state.captures, fetches: state.fetches }));
  assert.deepEqual(result, { status: 'unknown', current: null, latest: null });
  assert.deepEqual(state.captures.map(command => command.args), [['--version']]);
  assert.deepEqual(state.fetches, []);
  assert.deepEqual(state.reads, []);
  assert.deepEqual(state.runs, []);
});

test('unknown installed version cannot suppress a consented update', { timeout: 2000 }, async context => {
  const state = fixture();
  const result = await runClient(['claude', '--update', '--no-input'], state.deps);
  context.diagnostic(JSON.stringify({ result, output: state.output, errors: state.errors,
    runs: state.runs, captures: state.captures, fetches: state.fetches }));
  assert.equal(result, 0);
  assert.deepEqual(state.errors, []);
  assert.deepEqual(state.runs.map(command => [command.command, command.args]),
    [[state.executable, ['update']]]);
  assert.deepEqual(state.captures.map(command => [command.command, command.args]),
    [[state.executable, ['--version']]]);
  assert.deepEqual(state.fetches, []);
  assert.deepEqual(state.reads, []);
  assert.match(state.output.join(''), /The installed version could not be read\./);
  assert.doesNotMatch(state.output.join(''), /is the latest/);
});
