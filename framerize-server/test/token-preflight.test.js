import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { run } from '../dist/cli.js';
import { createServer } from '../dist/server.js';

test('CLI snapshots a private token before provider creation', async context => {
  const root = mkdtempSync(join(tmpdir(), 'server-token-preflight-'));
  context.after(() => rmSync(root, { recursive: true, force: true }));
  const file = join(root, 'token');
  const token = 'synthetic-local-token';
  writeFileSync(file, token, { mode: 0o600 });
  const events = new EventEmitter();
  events.stdout = events.stderr = { write() {} };
  let closed = 0;
  const result = await run(['check', '--server-token-file', file,
    '--upstream', 'http://127.0.0.1:1'], {
    env: {}, process: events, logger: { info() {} },
    createProvider(config) {
      assert.equal(config.serverToken, '');
      assert.ok(!JSON.stringify(config).includes(token));
      rmSync(file);
      return { close() { closed += 1; } };
    },
    createServer(config, dependencies) {
      assert.equal(config.serverToken, token);
      assert.equal(config.serverTokenFile, '');
      assert.ok(Object.isFrozen(config));
      assert.ok(!Object.keys(config).includes('serverToken'));
      assert.ok(!JSON.stringify(config).includes(token));
      return createServer(config, dependencies);
    },
  });
  assert.equal(result, 0);
  assert.equal(closed, 1);
  for (const name of ['SIGINT', 'SIGTERM']) assert.equal(events.listenerCount(name), 0);
});
