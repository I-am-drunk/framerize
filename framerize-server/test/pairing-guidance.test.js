import assert from 'node:assert/strict';
import { EventEmitter, once } from 'node:events';
import { mkdtemp, readdir, rm } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { run } from '../dist/cli.js';

async function fixture(action, json = false, failReady = false) {
  const root = await mkdtemp(join(tmpdir(), 'frm-pair-guidance-'));
  const probe = createServer();
  let output = '', failure, code, readyCalls = 0;
  const commands = [];
  const state = { running: false, enabled: false };
  const events = new EventEmitter();
  events.stdout = { write: value => { output += value; } };
  events.stderr = { write: () => assert.fail('Unexpected stderr') };
  try {
    probe.listen(0, '127.0.0.1');
    await once(probe, 'listening');
    const port = probe.address().port;
    await new Promise((resolve, reject) => probe.close(error => error ? reject(error) : resolve()));
    const env = { HOME: root, USERPROFILE: root, FRAMERIZE_SERVER_HOME: join(root, 'state'),
      XDG_CONFIG_HOME: join(root, 'xdg') };
    const argv = [action, '--host', '127.0.0.1', '--port', String(port),
      '--upstream', 'http://127.0.0.1:1', ...(action === 'up' ? ['--no-lan'] : []),
      ...(json ? ['--json'] : [])];
    try {
      code = await run(argv, { process: events, env, setup: {
        program: ['/synthetic/node', '/synthetic/framerize.js'], platform: 'linux', uid: 7342,
        ready: async () => {
          readyCalls++;
          if (failReady) throw new Error('synthetic readiness failure');
          return 7342;
        },
        run: async (command, args) => {
          commands.push([command, ...args]);
          if (command === 'loginctl') return { code: 0, stdout: 'yes\n' };
          assert.equal(command, 'systemctl');
          if (args.includes('show')) return { code: 0, stdout: state.running
            ? 'ActiveState=active\nSubState=running\nMainPID=7342\n'
            : 'ActiveState=inactive\nSubState=dead\nMainPID=0\n' };
          if (args.includes('is-enabled')) return { code: state.enabled ? 0 : 1, stdout: '' };
          if (args.includes('enable')) state.enabled = true;
          if (args.includes('restart')) state.running = true;
          if (args.includes('stop')) state.running = false;
          if (args.includes('disable')) state.enabled = false;
          return { code: 0, stdout: '' };
        },
      } });
    } catch (error) { failure = error; }
    assert.equal(events.listenerCount('SIGINT') + events.listenerCount('SIGTERM'), 0);
    if (action === 'pair') assert.deepEqual(await readdir(root), []);
    return { code, failure, output, commands, readyCalls, origin: `http://127.0.0.1:${port}` };
  } finally {
    if (probe.listening) await new Promise(resolve => probe.close(resolve));
    await rm(root, { recursive: true, force: true });
    await assert.rejects(readdir(root), { code: 'ENOENT' });
  }
}

for (const action of ['pair', 'up']) {
  test(`${action} names the receiving machine and the explicit Remote connect command`, { timeout: 10000 }, async () => {
    const result = await fixture(action);
    assert.equal(result.failure, undefined);
    assert.equal(result.code, 0);
    assert.equal(result.readyCalls, 1);
    assert.ok(result.output.endsWith('On the other machine, with framerize-remote installed:\n'
      + `  frm connect '${result.origin}/pair'\n`), result.output);
    assert.doesNotMatch(result.output, /^Next: frm '/m);
    if (action === 'pair') assert.deepEqual(result.commands, []);
  });

  test(`${action} keeps its structured pairing result`, { timeout: 10000 }, async () => {
    const result = await fixture(action, true);
    assert.equal(result.failure, undefined);
    assert.equal(result.code, 0);
    assert.equal(result.output, JSON.stringify({ origin: result.origin,
      pairingUrl: `${result.origin}/pair`, ready: true,
      ...(action === 'up' ? { service: 'installed' } : {}) }) + '\n');
  });
}

test('failed readiness prints no pairing invitation', { timeout: 10000 }, async () => {
  const result = await fixture('pair', false, true);
  assert.equal(result.failure?.message, 'synthetic readiness failure');
  assert.equal(result.output, '');
  assert.equal(result.readyCalls, 1);
  assert.deepEqual(result.commands, []);
});
