import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const launcher = fileURLToPath(new URL('../bin/framerize-server.js', import.meta.url));

function stateFor(context) {
  const state = mkdtempSync(join(tmpdir(), 'server-cli-configuration-'));
  context.after(() => rmSync(state, { recursive: true, force: true }));
  return state;
}

function launch(state, args, env = {}, file = launcher) {
  const result = spawnSync(process.execPath, [file, ...args], {
    cwd: state, encoding: 'utf8', timeout: 5000, killSignal: 'SIGKILL',
    env: {
      HOME: state, USERPROFILE: state, TMPDIR: state, TMP: state, TEMP: state,
      NODE_DISABLE_COMPILE_CACHE: '1',
      ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}),
      ...env,
    },
  });
  assert.equal(result.error, undefined);
  assert.equal(result.signal, null);
  assert.equal(result.stdout, '');
  assert.doesNotMatch(result.stderr, /canary/);
  return result;
}

const diagnostic = {
  event: 'server.failed', message: 'Server configuration is invalid.',
  next: 'Check option values, FRAMERIZE_SERVER_* settings and account session references. Run framerize-server --help for options. Environment guide: https://github.com/I-am-drunk/framerize/blob/main/framerize-server/docs/environment.md.',
};

const invalid = [
  ['port value', ['serve', '--port', 'port-canary'], {}],
  ['timeout range', ['check', '--inference-timeout-ms', '0'], {}],
  ['backend value', ['check', '--backend', 'backend-canary'], {}],
  ['credential references', ['check', '--authorization-env', 'SYNTHETIC_AUTH',
    '--authorization-file', '/path-canary'], {}],
  ['token environment', ['check'], { FRAMERIZE_SERVER_TOKEN: 'token canary' }],
];

for (const [name, args, env] of invalid) {
  test(`real CLI reports redacted configuration guidance for ${name}`, context => {
    const result = launch(stateFor(context), args, env);
    assert.equal(result.status, 1);
    assert.deepEqual(JSON.parse(result.stderr), diagnostic);
  });
}

const sessions = [
  ['unset environment', { env: 'SESSION_canary' }, {}],
  ['invalid environment', { env: 'SESSION_canary' }, { SESSION_canary: ' session-canary ' }],
  ['missing file', { file: 'missing-session-canary' }, {}],
];

for (const [name, session, env] of sessions) {
  test(`real CLI reports account session guidance for ${name}`, context => {
    const state = stateFor(context);
    const accounts = join(state, 'accounts-canary.json');
    writeFileSync(accounts, JSON.stringify([
      { id: 'account-canary', projectId: 'project-canary', session },
    ]), { mode: 0o600 });
    const result = launch(state, ['serve', '--backend', 'framer', '--accounts-file', accounts,
      '--host', '127.0.0.1', '--port', '0', '--framer-origin', 'http://127.0.0.1:1'],
    { FRAMERIZE_SERVER_TOKEN: 'token-canary', ...env });
    assert.equal(result.status, 1);
    assert.ok(!result.stderr.includes(state), 'Private state paths must not appear in diagnostics');
    assert.deepEqual(JSON.parse(result.stderr), diagnostic);
  });
}

test('real CLI accepts the same check after correcting its environment', context => {
  const state = stateFor(context);
  const args = ['check', '--backend', 'http'];
  const rejected = launch(state, args, { FRAMERIZE_SERVER_PORT: 'port-canary' });
  const repaired = launch(state, args, { FRAMERIZE_SERVER_PORT: '0' });
  assert.equal(rejected.status, 1);
  assert.equal(repaired.status, 0);
  assert.equal(JSON.parse(repaired.stderr).event, 'config.valid');
  assert.deepEqual(JSON.parse(rejected.stderr), diagnostic);
});

test('launcher keeps the generic fallback for an unrelated run error', context => {
  const state = stateFor(context);
  mkdirSync(join(state, 'bin'));
  mkdirSync(join(state, 'dist'));
  writeFileSync(join(state, 'package.json'), '{"type":"module"}\n');
  const copied = join(state, 'bin', 'framerize-server.js');
  copyFileSync(launcher, copied);
  const errors = pathToFileURL(join(dirname(launcher), '..', 'dist', 'errors.js')).href;
  writeFileSync(join(state, 'dist', 'errors.js'), `export * from ${JSON.stringify(errors)};\n`);
  writeFileSync(join(state, 'dist', 'cli.js'),
    "export async function run() { throw new Error('internal-canary'); }\n");
  const result = launch(state, ['check'], {}, copied);
  assert.equal(result.status, 1);
  assert.deepEqual(JSON.parse(result.stderr), {
    event: 'server.failed', message: 'Server failed to start or stop',
  });
});
