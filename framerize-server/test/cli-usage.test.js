import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const launcher = fileURLToPath(new URL('../bin/framerize-server.js', import.meta.url));

function launch(context, args, status) {
  const state = mkdtempSync(join(tmpdir(), 'server-cli-usage-'));
  context.after(() => rmSync(state, { recursive: true, force: true }));
  const result = spawnSync(process.execPath, [launcher, ...args], {
    cwd: state, encoding: 'utf8', timeout: 5000, killSignal: 'SIGKILL',
    env: {
      HOME: state, USERPROFILE: state, TMPDIR: state, TMP: state, TEMP: state,
      NODE_DISABLE_COMPILE_CACHE: '1',
      ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}),
    },
  });
  assert.equal(result.error, undefined);
  assert.equal(result.status, status);
  assert.equal(result.signal, null);
  return result;
}

const invalid = [
  ['unknown command', ['command-canary']],
  ['unsupported option', ['serve', '--server-token-file', '/path-canary',
    '--unsupported-canary', 'value-canary']],
  ['missing value', ['check', '--port']],
  ['unexpected positional argument', ['check', 'positional-canary']],
  ['option instead of value', ['serve', '--host', '--port', '0']],
];

for (const [name, args] of invalid) {
  test(`real CLI reports redacted usage guidance for ${name}`, context => {
    const result = launch(context, args, 1);
    assert.equal(result.stdout, '');
    assert.doesNotMatch(result.stderr, /canary/);
    assert.deepEqual(JSON.parse(result.stderr), {
      event: 'server.failed',
      message: 'Invalid command or option.',
      next: 'Run framerize-server --help for supported commands and options.',
    });
  });
}

test('real CLI provides help without startup', context => {
  const result = launch(context, ['--help'], 0);
  assert.match(result.stdout, /Usage: framerize-server \[serve\|check\|up\|pair\|status\|service\]/);
  assert.equal(result.stderr, '');
});

test('real CLI accepts a valid check command after correcting arguments', context => {
  const result = launch(context, ['check', '--backend', 'http', '--port', '0'], 0);
  assert.equal(result.stdout, '');
  assert.equal(JSON.parse(result.stderr).event, 'config.valid');
});
