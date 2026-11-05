import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { constants } from 'node:fs';
import { mkdir, mkdtemp, open, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const execute = promisify(execFile);
const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const guard = new URL('./helpers/reader-cli-preload.mjs', import.meta.url).href;

for (const kind of ['fifo', 'stable']) {
  test(`CLI version lookup completes with ${kind} settings`, {
    skip: process.platform === 'win32' && 'Uses a POSIX synthetic client', timeout: 10000,
  }, async t => {
    const root = await realpath(await mkdtemp(join(tmpdir(), 'frm-reader-cli-')));
    const home = join(root, 'home'), bin = join(home, '.local/bin');
    const state = join(root, 'state'), config = join(root, 'claude');
    const settings = join(config, 'settings.json');
    let fifoCreated = false;
    t.after(async () => {
      try {
        if (fifoCreated) {
          let probe;
          try {
            probe = await open(settings, constants.O_WRONLY | constants.O_NONBLOCK);
            assert.fail('Settings FIFO still has an open reader');
          } catch (error) { assert.equal(error.code, 'ENXIO'); }
          finally { await probe?.close(); }
        }
        t.diagnostic('CLI closed; no settings FIFO reader remains.');
      } finally { await rm(root, { recursive: true, force: true }); }
    });
    await Promise.all([bin, state, config].map(path => mkdir(path, { recursive: true })));
    const client = join(bin, 'claude'), calls = join(root, 'calls.jsonl');
    const fetches = join(root, 'fetches.jsonl');
    await writeFile(client, `#!${process.execPath}
const fs = require('node:fs');
fs.appendFileSync(process.env.FRM_TEST_CALLS, JSON.stringify(process.argv.slice(2)) + '\\n');
if (process.argv.length !== 3 || process.argv[2] !== '--version') process.exit(99);
process.stdout.write('2.1.300 (Claude Code)\\n');
`, { mode: 0o700 });
    if (kind === 'fifo') {
      await execute('mkfifo', ['-m', '600', settings], { timeout: 3000 });
      fifoCreated = true;
    } else {
      await writeFile(settings, '{"autoUpdatesChannel":"stable","label":"終🙂"}');
    }
    const token = 'synthetic-reader-cli-token';
    const profile = JSON.stringify({ remote: 'http://127.0.0.1:9', port: 8788,
      localToken: { kind: 'value', value: token } });
    await writeFile(join(state, 'framerize-remote.json'), profile, { mode: 0o600 });
    const env = { HOME: home, USERPROFILE: home, FRM_HOME: state, CLAUDE_CONFIG_DIR: config,
      PATH: bin, FRM_TEST_CLIENT: client, FRM_TEST_CALLS: calls, FRM_TEST_FETCHES: fetches,
      NODE_DISABLE_COMPILE_CACHE: '1' };
    const result = await execute(process.execPath, ['--import', guard,
      join(packageRoot, 'bin/frm.js'), 'client', 'claude', '--update', '--no-input'],
    { cwd: root, env, timeout: 4000, killSignal: 'SIGKILL', maxBuffer: 65536 });
    assert.match(result.stdout, /Version 2\.1\.300 is the latest\./);
    assert.equal(result.stderr, '');
    assert.ok(!result.stdout.includes(token));
    assert.deepEqual((await readFile(calls, 'utf8')).trim().split('\n').map(JSON.parse), [['--version']]);
    const channel = kind === 'stable' ? 'stable' : 'latest';
    assert.deepEqual((await readFile(fetches, 'utf8')).trim().split('\n').map(JSON.parse),
      [`https://downloads.claude.ai/claude-code-releases/${channel}`]);
    assert.equal(await readFile(join(state, 'framerize-remote.json'), 'utf8'), profile);
  });
}
