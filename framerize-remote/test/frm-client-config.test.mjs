import test from 'node:test';
import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createConnection } from 'node:net';
import { setTimeout as delay } from 'node:timers/promises';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const manifest = JSON.parse(await readFile(join(packageRoot, 'package.json'), 'utf8'));
assert.equal(typeof manifest.bin?.frm, 'string');
const launcher = resolve(packageRoot, manifest.bin.frm);

async function assertClosed(port) {
  for (let attempt = 0; attempt < 40; attempt++) {
    const code = await new Promise(resolve => {
      const socket = createConnection({ host: '127.0.0.1', port });
      socket.once('connect', () => { socket.destroy(); resolve('open'); });
      socket.once('error', error => { socket.destroy(); resolve(error.code); });
    });
    if (code === 'ECONNREFUSED') return;
    await delay(20);
  }
  assert.fail('the timed-out version probe kept its listener open');
}

async function fixture(context) {
  const root = await mkdtemp(join(tmpdir(), 'frm-client-config-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  const env = { HOME: root, USERPROFILE: root, TMPDIR: root, TMP: root, TEMP: root,
    PATH: '', FRM_HOME: join(root, 'default'), CLAUDE_CONFIG_DIR: join(root, 'claude') };
  const custom = join(root, 'selected profile', 'remote.json');
  for (const [path, port] of [[join(env.FRM_HOME, 'framerize-remote.json'), 17001], [custom, 17002]]) {
    await mkdir(dirname(path), { recursive: true });
    await writeFile(path, JSON.stringify({ remote: 'http://127.0.0.1:43123', port,
      localToken: { kind: 'value', value: `synthetic-${port}` } }));
  }
  const run = (args, timeout = 5000) => new Promise(resolve => execFile(process.execPath, [launcher, 'client', ...args],
    { cwd: root, env, timeout, killSignal: 'SIGKILL', maxBuffer: 65536 },
    (error, stdout, stderr) => resolve({ code: error ? error.code ?? error.signal ?? 1 : 0, stdout, stderr })));
  return { root, env, custom, run };
}

test('client --config selects that profile and bypasses an invalid FRM_HOME',
  { timeout: 8000 }, async context => {
    const { env, custom, run } = await fixture(context);
    env.FRM_HOME = 'invalid-relative-home';
    const result = await run(['claude', '--config', custom, '--check']);
    assert.equal(result.code, 1, result.stderr);
    assert.match(result.stdout, /Remote base URL: http:\/\/127\.0\.0\.1:17002\/anthropic/);
    assert.match(result.stdout, /Local token: set/);
    assert.doesNotMatch(result.stdout + result.stderr, /synthetic-17002/);
  });

test('client without --config still reads the default profile', { timeout: 8000 }, async context => {
  const { run } = await fixture(context);
  const result = await run(['codex', '--check']);
  assert.equal(result.code, 1, result.stderr);
  assert.match(result.stdout, /Remote base URL: http:\/\/127\.0\.0\.1:17001\/openai\/v1/);
});

test('client check bounds a hung real version probe and closes its listener',
  { skip: process.platform === 'win32', timeout: 10000 }, async context => {
    const { root, env, run } = await fixture(context);
    const bin = join(root, 'bin');
    await mkdir(bin);
    env.PATH = bin;
    env.FRM_TEST_PROBE_STATE = join(root, 'probe.json');
    const descendant = `const net = require('node:net'), fs = require('node:fs');
      net.createServer(socket => socket.end()).listen(0, '127.0.0.1', function () {
        fs.writeFileSync(process.env.FRM_TEST_PROBE_STATE,
          JSON.stringify({ pid: process.pid, port: this.address().port }));
        process.send('ready');
      });`;
    await writeFile(join(bin, 'claude'), `#!${process.execPath}
const child = require('node:child_process').spawn(process.execPath,
  ['-e', ${JSON.stringify(descendant)}], { stdio: ['ignore', 'inherit', 'ignore', 'ipc'] });
child.once('message', () => process.exit(0));
`, { mode: 0o700 });
    let probe;
    context.after(async () => {
      if (probe) {
        try { process.kill(probe.pid, 'SIGKILL'); } catch {}
        await assertClosed(probe.port);
      }
    });
    const result = await run(['claude', '--check'], 8000);
    probe = JSON.parse(await readFile(env.FRM_TEST_PROBE_STATE, 'utf8'));
    assert.equal(result.code, 0, result.stderr);
    assert.match(result.stdout, /Version: unavailable/);
    assert.doesNotMatch(result.stdout + result.stderr, /synthetic-17001/);
    await assertClosed(probe.port);
  });

test('persist and undo share the explicitly selected profile directory',
  { timeout: 10000 }, async context => {
    const { env, custom, run } = await fixture(context);
    const settingsPath = join(env.CLAUDE_CONFIG_DIR, 'settings.json');
    await mkdir(env.CLAUDE_CONFIG_DIR);
    await writeFile(settingsPath, '{"model":"synthetic-model"}');
    const persisted = await run(['claude', '--config', custom, '--persist']);
    assert.equal(persisted.code, 0, persisted.stderr);
    const settings = JSON.parse(await readFile(settingsPath, 'utf8'));
    assert.deepEqual(settings, { model: 'synthetic-model', env: {
      ANTHROPIC_BASE_URL: 'http://127.0.0.1:17002/anthropic', ANTHROPIC_AUTH_TOKEN: 'synthetic-17002' } });
    assert.match(persisted.stdout, /--undo/);
    assert.match(persisted.stdout, /--config/);
    assert.doesNotMatch(persisted.stdout + persisted.stderr, /synthetic-17002/);
    await readFile(join(dirname(custom), 'claude-settings.json'));
    await assert.rejects(readFile(join(env.FRM_HOME, 'claude-settings.json')), { code: 'ENOENT' });
    await rm(custom);
    env.FRM_HOME = 'invalid-relative-home';
    const undone = await run(['claude', '--config', custom, '--undo']);
    assert.equal(undone.code, 0, undone.stderr);
    assert.deepEqual(JSON.parse(await readFile(settingsPath, 'utf8')), { model: 'synthetic-model' });
    await assert.rejects(readFile(join(dirname(custom), 'claude-settings.json')), { code: 'ENOENT' });
  });
