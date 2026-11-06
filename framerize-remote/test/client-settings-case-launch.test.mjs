import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { build } from 'esbuild';

const sourceRoot = resolve(process.env.FRAMERIZE_REMOTE_SOURCE_ROOT
  ?? fileURLToPath(new URL('../src/', import.meta.url)));
const compiled = await build({ absWorkingDir: sourceRoot, entryPoints: ['compatibility/command.ts'],
  bundle: true, write: false, metafile: true, platform: 'node', format: 'esm' });
const bytes = compiled.outputFiles[0].contents;
const { runClient } = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
const digest = value => createHash('sha256').update(value).digest('hex');
const inputs = await Promise.all(Object.keys(compiled.metafile.inputs).sort().map(async path =>
  ({ path, sha256: digest(await readFile(resolve(sourceRoot, path))) })));
console.log(JSON.stringify({ runtime: process.version, bundle: digest(bytes), inputs }));

const home = join('/synthetic', 'home');
const userPath = join(home, '.claude', 'settings.json');
const managedPath = join('/synthetic/programs', 'ClaudeCode', 'managed-settings.json');
const origin = 'http://127.0.0.1:8788';
const localToken = 'synthetic-local-token';
const executable = 'C:\\synthetic\\bin\\claude.EXE';
const passthrough = ['--model', 'synthetic-model'];
const expectedEnv = { ANTHROPIC_BASE_URL: `${origin}/anthropic`, ANTHROPIC_AUTH_TOKEN: localToken };

async function invoke(path, settings) {
  const commands = [], stdout = [], stderr = [], forbidden = [];
  const env = Object.freeze({ PATH: 'C:\\synthetic\\bin', PATHEXT: '.EXE', PROGRAMFILES: '/synthetic/programs' });
  const original = { ...env };
  const signals = ['SIGINT', 'SIGTERM'].map(signal => process.listenerCount(signal));
  const forbid = operation => { forbidden.push(operation); throw new Error(`Unexpected ${operation}`); };
  const code = await runClient(['claude', '--', ...passthrough], {
    platform: 'win32', env, home, cwd: '/synthetic/project', stateDir: '/synthetic/state',
    target: { origin, localToken },
    runner: {
      run: async (command, inherited) => { commands.push({ command, inherited }); return 0; },
      capture: async () => forbid('capture'),
    },
    io: { stdout: text => stdout.push(text), stderr: text => stderr.push(text) },
    probe: candidate => candidate === executable,
    read: async candidate => candidate === path ? JSON.stringify(settings) : null,
    fetch: async () => forbid('fetch'), prompt: async () => forbid('prompt'),
    realPath: () => forbid('realpath'), fileOwner: async () => forbid('fileOwner'),
  });
  assert.deepEqual(forbidden, []);
  assert.deepEqual(env, original);
  assert.deepEqual(['SIGINT', 'SIGTERM'].map(signal => process.listenerCount(signal)), signals);
  return { code, commands, stdout: stdout.join(''), stderr: stderr.join('') };
}

function assertLaunch(result, args) {
  assert.equal(result.code, 0);
  assert.equal(result.stderr, '');
  assert.equal(result.commands.length, 1);
  assert.equal(result.commands[0].command.command, executable);
  assert.deepEqual(result.commands[0].command.args, args);
  assert.deepEqual(result.commands[0].command.env, expectedEnv);
}

for (const [name, path, key, value] of [
  ['user token', userPath, 'anthropic_auth_token', 'synthetic-other-token'],
  ['user cloud provider', userPath, 'Claude_Code_Use_Vertex', '1'],
  ['managed base URL', managedPath, 'Anthropic_Base_Url', 'https://synthetic.invalid'],
]) {
  test(`Windows ${name} conflict prevents client launch`, { timeout: 3000 }, async () => {
    const result = await invoke(path, { env: { [key]: value } });
    assert.equal(result.code, 1);
    assert.deepEqual(result.commands, []);
    assert.equal(result.stdout, '');
    assert.ok(result.stderr.includes(`${path}: env.${key}`));
    assert.ok(result.stderr.includes('so it was not started:'));
    assert.ok(result.stderr.includes('frm client claude --undo'));
    assert.ok(!result.stderr.includes(localToken));
    if (value !== '1') assert.ok(!result.stderr.includes(value));
  });
}

test('Windows ordinary base URL receives only the existing launch override', { timeout: 3000 }, async () => {
  const result = await invoke(userPath, { env: { Anthropic_Base_Url: 'https://synthetic.invalid' } });
  const settings = JSON.stringify({ env: { ANTHROPIC_BASE_URL: expectedEnv.ANTHROPIC_BASE_URL } });
  assertLaunch(result, ['--settings', settings, ...passthrough]);
  assert.ok(!result.commands[0].command.args.join(' ').includes(localToken));
});

test('Windows matching settings launch with unchanged model arguments', { timeout: 3000 }, async () => {
  const result = await invoke(userPath, { env: { Anthropic_Base_Url: expectedEnv.ANTHROPIC_BASE_URL,
    anthropic_auth_token: localToken } });
  assertLaunch(result, passthrough);
});
