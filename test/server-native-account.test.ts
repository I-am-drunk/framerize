import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createServer } from 'node:http';
import { once } from 'node:events';
import test from 'node:test';
import { checkNativeAccount } from '../scripts/native-account-smoke.js';
import { accountServerFixture } from '../scripts/fixtures/installed-server.js';
import { exerciseAccountFlow } from '../scripts/fixtures/account-scenario.js';

test('native account fixture uses selected references without secrets in arguments', async () => {
  const root = mkdtempSync(join(tmpdir(), 'native-account-policy-'));
  try {
    const config = accountServerFixture(root, {}, 'http://127.0.0.1:1');
    assert.doesNotMatch(config.args.join(' '), /synthetic-account|synthetic-server-credential/);
    assert.equal(config.env.FIXTURE_ACCOUNT_SESSION, 'session=synthetic-account');
    assert.equal(config.args[config.args.indexOf('--account-id') + 1], 'selected');
    const accounts = readFileSync(join(root, 'accounts.json'), 'utf8');
    assert.doesNotMatch(accounts, /session=|synthetic-/);
    assert.deepEqual(JSON.parse(accounts).map((account: { id: string }) => account.id), ['unused', 'selected']);
    await assert.rejects(checkNativeAccount(root), /regular file/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});

for (const mode of ['client-cancel', 'upstream-close', 'empty-body-close'] as const) {
  test(`account cancellation evidence distinguishes ${mode}`, { timeout: 5000 }, async () => {
    let closed!: () => void;
    const scenario = { url: '', state: { auth: 0, inference: 0, cancelled: 0, failures: [] as unknown[] },
      cancellation: new Promise<void>(resolve => { closed = resolve; }), async close() {} };
    const server = createServer((request, response) => {
      request.resume();
      request.on('end', () => {
        if (request.url === '/ready') {
          const allowed = request.headers.authorization === 'Bearer synthetic-client';
          if (allowed) scenario.state.auth = 1;
          response.writeHead(allowed ? 200 : 401); response.end('{}'); return;
        }
        scenario.state.inference += 1;
        if (scenario.state.inference === 1) {
          response.end('data: {"choices":[{"delta":{"content":"fixture"}}]}\n\ndata: [DONE]\n\n');
          return;
        }
        response.once('close', () => { scenario.state.cancelled += 1; closed(); });
        if (mode === 'client-cancel') {
          response.writeHead(200, { 'content-type': 'text/event-stream' });
          response.write('data: waiting\n\n');
        } else if (mode === 'empty-body-close') {
          response.writeHead(200, { 'content-type': 'text/event-stream' });
          response.flushHeaders();
          setImmediate(() => response.destroy());
        } else response.destroy();
      });
    });
    try {
      server.listen(0, '127.0.0.1');
      await once(server, 'listening');
      const address = server.address();
      assert.ok(address && typeof address !== 'string');
      const flow = exerciseAccountFlow(`http://127.0.0.1:${address.port}`, 'synthetic-client', scenario);
      if (mode === 'client-cancel') assert.equal((await flow).cancelled, true);
      else await assert.rejects(flow);
      assert.equal(scenario.state.inference, 2);
      assert.equal(scenario.state.cancelled, 1);
    } finally {
      server.closeAllConnections();
      await new Promise<void>(resolve => server.close(() => resolve()));
    }
  });
}
