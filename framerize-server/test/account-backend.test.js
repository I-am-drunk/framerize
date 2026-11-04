import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer as httpServer } from 'node:http';
import { once } from 'node:events';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createAccountBackend } from '../dist/account-backend.js';
import { createServer } from '../dist/server.js';
import { collect, deferred } from './helpers/fixtures.js';

const records = ['personal', 'work'].map(id => ({ id, projectId: `project-${id}`, session: { env: id.toUpperCase() } }));
const environment = { PERSONAL: 'session=synthetic-personal', WORK: 'session=synthetic-work' };

async function fixture(context, responseDelayMs = 0) {
  const directory = await mkdtemp(join(tmpdir(), 'framerize-account-backend-'));
  const accountsFile = join(directory, 'accounts.json');
  await writeFile(accountsFile, JSON.stringify(records));
  const requests = [];
  const gateway = httpServer(async (request, response) => {
    const bytes = await collect(request);
    requests.push({ path: request.url, headers: request.headers, bytes });
    response.setHeader('content-type', 'application/json');
    if (request.url === '/auth/ai/access-token') {
      const account = request.headers.cookie === environment.PERSONAL ? 'personal' : 'work';
      response.end(JSON.stringify({ accessToken: `token-${account}`, expiresAt: new Date(Date.now() + 600_000).toISOString() }));
    } else if (responseDelayMs) {
      const timer = setTimeout(() => response.end('{"ok":true}'), responseDelayMs);
      response.once('close', () => clearTimeout(timer));
    } else response.end('{"ok":true}');
  });
  gateway.listen(0, '127.0.0.1'); await once(gateway, 'listening');
  const made = [];
  const minted = [];
  const closed = [];
  const backends = [];
  const options = { accountsFile, env: environment,
    origin: `http://127.0.0.1:${gateway.address().port}`,
    createMinter({ accountId }) {
      made.push(accountId);
      return { async mint({ signal }) {
        signal.throwIfAborted(); minted.push(accountId); return { token: `castle-${accountId}` };
      }, async close() { closed.push(accountId); } };
    },
  };
  context.after(async () => {
    await Promise.all(backends.map(backend => backend.close()));
    gateway.closeAllConnections(); await new Promise(resolve => gateway.close(resolve));
    await rm(directory, { recursive: true, force: true });
  });
  return { options, requests, made, minted, closed, directory,
    async backend(extra = {}) {
      const result = await createAccountBackend({ ...options, ...extra });
      backends.push(result); return result;
    },
  };
}

const contextSignal = () => ({ signal: new AbortController().signal });

test('account backend propagates the inference deadline', { timeout: 3000 }, async context => {
  const setup = await fixture(context, 300);
  const backend = await setup.backend({ accountId: 'personal', inferenceTimeoutMs: 60 });
  await assert.rejects(backend.forward(request()), { code: 'headers_timeout', status: 504 });
});
const request = () => ({ ...contextSignal(), path: '/v1/responses', headers: {},
  body: (async function* () { yield Buffer.from('{"model":"example","input":"hello"}'); })(),
});

test('JSON accounts keep session caches, project bindings and minters isolated through the listener', async context => {
  const setup = await fixture(context);
  const servers = [];
  context.after(async () => { for (const server of servers) await server.close(); });
  for (const accountId of ['personal', 'work']) {
    const provider = await setup.backend({ accountId });
    const app = await createServer({ port: 0, serverToken: 'local-only', accountsFile: setup.options.accountsFile }, { provider });
    servers.push(app);
    const address = await app.listen();
    const response = await fetch(`http://127.0.0.1:${address.port}/v1/responses`, {
      method: 'POST', headers: { authorization: 'Bearer local-only', 'content-type': 'application/json' },
      body: '{"model":"example","input":"hello"}',
    });
    assert.equal(response.status, 200); assert.deepEqual(await response.json(), { ok: true });
    const sent = setup.requests.at(-1);
    assert.equal(sent.path, '/ai/v3/openai/responses/');
    assert.equal(sent.headers.authorization, `Bearer token-${accountId}`);
    assert.equal(sent.headers.cookie, undefined);
    assert.equal(JSON.parse(sent.bytes).projectId, `project-${accountId}`);
    assert.equal(JSON.parse(sent.bytes).castleToken, `castle-${accountId}`);
  }
  assert.deepEqual(setup.made, ['personal', 'work']);
  assert.deepEqual(setup.minted, ['personal', 'work']);
  await servers[0].close();
  assert.deepEqual(setup.closed, ['personal']);
  assert.equal((await fetch(`http://127.0.0.1:${servers[1].server.address().port}/ready`, {
    headers: { authorization: 'Bearer local-only' },
  }).then(response => response.json())).status, 'ready');
  assert.equal(setup.requests.filter(item => item.path === '/auth/ai/access-token').length, 2);
});

test('account Castle device IDs stay bound across repeated mints', async context => {
  const setup = await fixture(context);
  await writeFile(setup.options.accountsFile, JSON.stringify(records.map(account => ({
    ...account, castleDeviceId: `synthetic-device-${account.id}`,
  }))));
  const observed = [];
  for (const accountId of ['personal', 'work']) {
    const backend = await setup.backend({ accountId, createMinter: () => ({
      async mint({ signal, cuid }) {
        signal.throwIfAborted(); observed.push({ accountId, cuid });
        return { token: `castle-${accountId}` };
      }, async close() {},
    }) });
    for (const attempt of [1, 2]) {
      const response = await backend.forward(request());
      await collect(response.body);
      assert.equal(observed.length, (accountId === 'personal' ? 0 : 2) + attempt);
    }
  }
  assert.deepEqual(observed, ['personal', 'personal', 'work', 'work'].map(accountId => ({
    accountId, cuid: `synthetic-device-${accountId}`,
  })));
});

test('separate instances do not share even the same account token cache', async context => {
  const setup = await fixture(context);
  const first = await setup.backend({ accountId: 'personal' });
  const second = await setup.backend({ accountId: 'personal' });
  assert.equal(setup.requests.length, 0);
  await first.ready(contextSignal()); await second.ready(contextSignal());
  assert.equal(setup.requests.length, 2);
  await first.close();
  assert.equal(await second.ready(contextSignal()), true);
  assert.equal(setup.requests.length, 2);
  assert.equal(setup.minted.length, 0);
});

test('only the selected account session is read', async context => {
  const setup = await fixture(context);
  const env = { PERSONAL: environment.PERSONAL, get WORK() { throw new Error('synthetic-private-other'); } };
  const backend = await setup.backend({ accountId: 'personal', env });
  assert.equal(await backend.ready(contextSignal()), true);
  assert.equal(setup.requests[0].headers.cookie, environment.PERSONAL);
  await assert.rejects(setup.backend({ accountId: 'work', env }), { message: 'Framer account setup failed' });
  assert.deepEqual(setup.made, ['personal']);
});

test('a relative private session file is resolved from JSON, not the working directory', async context => {
  const setup = await fixture(context);
  await writeFile(join(setup.directory, 'private-session'), environment.PERSONAL + '\n', { mode: 0o600 });
  await writeFile(setup.options.accountsFile, JSON.stringify([
    { ...records[0], session: { file: 'private-session' } },
  ]));
  const backend = await setup.backend({ env: {} });
  const response = await backend.forward(request());
  assert.deepEqual(JSON.parse((await collect(response.body)).toString()), { ok: true });
  assert.equal(setup.requests[0].headers.cookie, environment.PERSONAL);
});

test('ambiguous, disabled and missing accounts fail before credential access or minter creation', async context => {
  const setup = await fixture(context);
  let reads = 0;
  const env = new Proxy({}, { get() { reads += 1; return environment.PERSONAL; } });
  await assert.rejects(setup.backend({ env }), { message: 'Framer account setup failed' });
  await assert.rejects(setup.backend({ env, accountId: 'absent' }), { message: 'Framer account setup failed' });
  await writeFile(setup.options.accountsFile, JSON.stringify([{ ...records[0], enabled: false }]));
  await assert.rejects(setup.backend({ env, accountId: 'personal' }), { message: 'Framer account setup failed' });
  assert.equal(setup.made.length, 0);
  assert.equal(setup.requests.length, 0);
  assert.equal(reads, 0);
});

test('invalid Castle device identity fails before account session access', async context => {
  const setup = await fixture(context);
  await writeFile(setup.options.accountsFile, JSON.stringify([
    { ...records[0], castleDeviceId: 'invalid private value' },
  ]));
  let reads = 0;
  const env = new Proxy({}, { get() { reads += 1; return environment.PERSONAL; } });
  await assert.rejects(setup.backend({ accountId: 'personal', env }),
    { message: 'Framer account setup failed' });
  assert.equal(reads, 0);
  assert.equal(setup.made.length, 0);
  assert.equal(setup.requests.length, 0);
});

test('pre-aborted startup reads no credentials and creates no minter', async context => {
  const setup = await fixture(context);
  let reads = 0;
  const env = new Proxy({}, { get() { reads += 1; return environment.PERSONAL; } });
  await assert.rejects(setup.backend({ accountId: 'personal', env, signal: AbortSignal.abort('synthetic-private') }),
    { code: 'request_cancelled', message: 'Framer account startup cancelled' });
  assert.equal(reads, 0);
  assert.equal(setup.made.length, 0);
});

test('startup cancellation waits for a late minter to close before rejecting', { timeout: 5000 }, async context => {
  const setup = await fixture(context);
  const controller = new AbortController();
  const entered = deferred();
  const created = deferred();
  const closing = deferred();
  const closed = deferred();
  let settled = false;
  let closeCalls = 0;
  const checked = assert.rejects(setup.backend({ accountId: 'personal', signal: controller.signal,
    async createMinter() { entered.resolve(); return created.promise; },
  }).finally(() => { settled = true; }), { code: 'request_cancelled' });
  await entered.promise;
  controller.abort('synthetic-private');
  created.resolve({ async mint() { assert.fail('must not mint'); },
    async close() { closeCalls += 1; closing.resolve(); await closed.promise; } });
  await closing.promise;
  assert.equal(settled, false);
  closed.resolve(); await checked;
  assert.equal(closeCalls, 1);
  assert.equal(setup.requests.length, 0);
});

test('minter initialization and invalid contracts fail without exposing private errors', async context => {
  const setup = await fixture(context);
  let closed = 0;
  await assert.rejects(setup.backend({ accountId: 'personal', createMinter() {
    throw new Error('synthetic-private-minter');
  } }), { message: 'Framer account setup failed' });
  await assert.rejects(setup.backend({ accountId: 'personal', createMinter() {
    return { async close() { closed += 1; } };
  } }), { message: 'Framer account setup failed' });
  assert.equal(closed, 1);
  assert.equal(setup.requests.length, 0);
});

for (const value of [null, {}, { get accountsFile() { throw new Error('synthetic-private'); } },
  { accountsFile: 'absent', origin: 'http://example.test', createMinter() {} }]) {
  test('invalid account backend options return fixed errors', async () => {
    await assert.rejects(createAccountBackend(value), { message: 'Invalid Framer account configuration' });
  });
}
