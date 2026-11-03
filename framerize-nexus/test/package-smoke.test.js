import test from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const source = readFileSync(new URL('../scripts/package-smoke.js', import.meta.url), 'utf8');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
const loadScript = (text) => new AsyncFunction('assert', 'execFileSync', 'mkdtempSync',
  'readdirSync', 'rmSync', 'tmpdir', 'path', 'pathToFileURL', 'process', 'console',
  text.replace(/^import .*;\r?\n/gm, ''));

async function fixture(context, { mode = 'relative', failure = '', platform = 'linux', text = source } = {}) {
  const root = mkdtempSync(path.join(tmpdir(), 'nexus-package-cleanup-test-'));
  context.after(() => rmSync(root, { recursive: true, force: true }));
  const name = 'framerize-nexus-0.1.9.tgz';
  const input = path.join(root, name);
  writeFileSync(input, 'original tarball');
  const args = mode === 'generated' ? [] : mode === 'downloaded' ? ['--downloaded']
    : [mode === 'absolute' ? input : name];
  const calls = [];
  const removed = [];
  let allocated;
  const command = (executable, argv, options) => {
    const npmArgs = executable === '/fixture/node' ? argv.slice(1) : argv;
    const action = npmArgs[0];
    calls.push(action);
    if (failure === action) throw new Error(`${action} failure`);
    if (action === 'pack') {
      assert.equal(npmArgs[2], allocated);
      writeFileSync(path.join(allocated, name), 'generated tarball');
      return name;
    }
    assert.equal(options.cwd, allocated);
    if (action === 'install') {
      assert.equal(npmArgs.at(-1), mode === 'generated' ? path.join(allocated, name) : input);
      const installed = path.join(allocated, 'node_modules', 'framerize-nexus', 'src');
      mkdirSync(installed, { recursive: true });
      writeFileSync(path.join(installed, 'config.js'), failure === 'import'
        ? "throw new Error('import failure');" : 'module.exports = {};');
    }
    return 'Framerize Nexus';
  };
  let error;
  try {
    await loadScript(text)(assert, command, (prefix) => {
      allocated = mkdtempSync(prefix);
      return allocated;
    }, () => failure === 'missing' ? [] : failure === 'duplicate'
      ? [name, 'framerize-nexus-other.tgz'] : [name], (directory, options) => {
      removed.push(directory);
      rmSync(directory, options);
    }, () => root, { ...path, resolve: (...parts) => path.resolve(root, ...parts) },
    pathToFileURL, { argv: ['node', 'package-smoke.js', ...args], platform,
      execPath: '/fixture/node', env: { npm_execpath: '/fixture/npm-cli.js' } }, { log() {} });
  } catch (caught) {
    error = caught;
  }
  assert.deepEqual(removed, [allocated]);
  assert.equal(existsSync(allocated), false);
  assert.equal(readFileSync(input, 'utf8'), 'original tarball');
  if (mode !== 'generated') assert.ok(!calls.includes('pack'));
  if (failure === 'missing' || failure === 'duplicate') assert.deepEqual(calls, []);
  return error;
}

for (const platform of ['linux', 'darwin', 'win32']) {
  for (const mode of ['generated', 'relative', 'absolute', 'downloaded']) {
    test(`${platform}: cleans ${mode} package smoke without deleting the input`, async (context) => {
      assert.equal(await fixture(context, { platform, mode }), undefined);
    });
  }
  for (const failure of ['pack', 'install', '--help', 'exec', 'import', 'missing', 'duplicate']) {
    test(`${platform}: removes partial package smoke after ${failure} failure`, async (context) => {
      const mode = failure === 'pack' ? 'generated'
        : ['missing', 'duplicate'].includes(failure) ? 'downloaded' : 'relative';
      const error = await fixture(context, { platform, mode, failure });
      assert.equal(error?.message, ['missing', 'duplicate'].includes(failure)
        ? 'exactly one downloaded Nexus tarball is required' : `${failure} failure`);
    });
  }
}

test('package smoke supports a CRLF checkout', async (context) => {
  assert.equal(await fixture(context, { platform: 'win32',
    text: source.replace(/\r?\n/g, '\r\n') }), undefined);
});
