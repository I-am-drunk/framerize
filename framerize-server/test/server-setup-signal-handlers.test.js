import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { mkdtemp, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { pathToFileURL } from 'node:url';

const base = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ? pathToFileURL(join(process.env.FRAMERIZE_SERVER_PACKAGE_ROOT, 'dist') + '/')
  : new URL('../dist/', import.meta.url);
const { run } = await import(new URL('cli.js', base));
const { runSetupCommand } = await import(new URL('setup.js', base));

for (const signal of ['SIGINT', 'SIGTERM']) test(`setup removes only its own ${signal} listener`, async t => {
  const root = await mkdtemp(join(tmpdir(), 'server-signal-handlers-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const events = new EventEmitter();
  events.stdout = events.stderr = { write() {} };
  const existing = () => {};
  for (const event of ['SIGINT', 'SIGTERM']) events.on(event, existing);
  await assert.rejects(run(['up', '--no-lan'], {
    process: events, env: { HOME: root, FRAMERIZE_SERVER_HOME: join(root, 'state') },
    ask: async () => { events.emit(signal); return 'http://127.0.0.1:1'; },
    setup: { platform: 'linux', run: async () => assert.fail('No command should start') },
  }), /Server setup was cancelled/);
  for (const event of ['SIGINT', 'SIGTERM']) assert.deepEqual(events.listeners(event), [existing]);
  await assert.rejects(readdir(join(root, 'state')), { code: 'ENOENT' });
});

test('already cancelled setup performs no work', async t => {
  const root = await mkdtemp(join(tmpdir(), 'server-pre-cancel-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const reason = new Error('cancelled before setup');
  await assert.rejects(runSetupCommand(['up'], {
    env: { HOME: root, FRAMERIZE_SERVER_HOME: join(root, 'state') },
    signal: AbortSignal.abort(reason),
    stdout: { write: () => assert.fail('No output expected') },
    run: async () => assert.fail('No command expected'),
  }), error => error === reason);
});
