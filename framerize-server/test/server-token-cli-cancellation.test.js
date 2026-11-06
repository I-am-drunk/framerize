import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { EventEmitter } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { run } from '../dist/cli.js';

async function bounded(pending, milliseconds = 1500) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Token fixture timed out')), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

test('check still validates a token file and closes the created server', async t => {
  const root = await fs.mkdtemp(join(tmpdir(), 'server-token-check-'));
  t.after(async () => {
    await fs.rm(root, { recursive: true, force: true });
    await assert.rejects(fs.stat(root), { code: 'ENOENT' });
  });
  const tokenPath = join(root, 'token');
  await fs.writeFile(tokenPath, 'synthetic-normal-token\n', { mode: 0o600 });
  const events = new EventEmitter();
  let output = '', closes = 0;
  events.stdout = events.stderr = { write(text) { output += text; } };
  assert.equal(await run(['check', '--server-token-file', tokenPath], {
    process: events,
    env: { HOME: root, USERPROFILE: root, FRAMERIZE_SERVER_HOME: join(root, 'state') },
    createProvider() { assert.fail('No upstream was configured'); },
    createServer(config) {
      assert.equal(config.serverToken, 'synthetic-normal-token');
      assert.equal(config.serverTokenFile, '');
      return { close() { closes++; } };
    },
  }), 0);
  assert.equal(closes, 1);
  assert.match(output, /config.valid/);
  for (const name of ['SIGINT', 'SIGTERM']) assert.equal(events.listenerCount(name), 0);
});

for (const command of ['serve', 'check']) for (const signal of ['SIGINT', 'SIGTERM']) {
  test(`${command} handles ${signal} while opening its token file`, { timeout: 5000 }, async t => {
    const root = await fs.mkdtemp(join(tmpdir(), 'server-token-cli-'));
    const tokenPath = join(root, 'token');
    const token = 'synthetic-cli-token\n';
    const held = Promise.withResolvers();
    const entered = Promise.withResolvers();
    const closed = Promise.withResolvers();
    const events = new EventEmitter();
    const startupCalls = [];
    let output = '';
    events.stdout = events.stderr = { write(text) { output += text; } };
    const originalOpen = fs.open;
    let pending, handle, mock, closes = 0, requested = false;
    t.after(async () => {
      held.resolve();
      try {
        if (pending) await bounded(pending);
        if (requested) await bounded(closed.promise);
      } finally {
        mock?.mock.restore();
        syncBuiltinESMExports();
        if (handle && handle.fd !== -1) await handle.close();
        await fs.rm(root, { recursive: true, force: true });
        await assert.rejects(fs.stat(root), { code: 'ENOENT' });
      }
    });
    await fs.writeFile(tokenPath, token, { mode: 0o600 });
    mock = t.mock.method(fs, 'open', async (path, ...args) => {
      if (path !== tokenPath) return originalOpen(path, ...args);
      requested = true;
      entered.resolve();
      await held.promise;
      handle = await originalOpen(path, ...args);
      const originalClose = handle.close.bind(handle);
      t.mock.method(handle, 'close', async () => {
        await originalClose();
        closes++;
        closed.resolve();
      });
      return handle;
    });
    syncBuiltinESMExports();
    pending = run([command, '--server-token-file', tokenPath], {
      process: events,
      env: { HOME: root, USERPROFILE: root, FRAMERIZE_SERVER_HOME: join(root, 'state') },
      createProvider() { startupCalls.push('provider'); assert.fail('Cancelled startup'); },
      createServer() { startupCalls.push('server'); assert.fail('Cancelled startup'); },
    }).then(value => ({ value }), error => ({ error }));
    await bounded(entered.promise);
    events.emit(signal);
    let outcome;
    try { outcome = await bounded(pending, 400); }
    finally { held.resolve(); }
    await bounded(closed.promise);
    assert.deepEqual(outcome, { value: 0 }, 'Keep the normal CLI shutdown exit code');
    assert.equal(closes, 1);
    assert.equal(handle.fd, -1);
    assert.equal(await fs.readFile(tokenPath, 'utf8'), token);
    assert.equal(output, '', 'Cancelled startup must not report readiness or validity');
    assert.deepEqual(startupCalls, []);
    for (const name of ['SIGINT', 'SIGTERM']) assert.equal(events.listenerCount(name), 0);
    await assert.rejects(fs.stat(join(root, 'state')), { code: 'ENOENT' });
  });
}
