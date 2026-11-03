import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs, run } from '../dist/cli.js';

test('CLI selects the account provider without a generic upstream', async () => {
  let made = 0; let closed = 0;
  const env = { SESSION: 'synthetic-session' };
  const result = await run(['check', '--backend', 'framer', '--accounts-file', '/synthetic/accounts.json',
    '--account-id', 'selected'], { env, logger: { info() {} },
    createProvider(config, context) {
      assert.equal(config.backend, 'framer');
      assert.equal(config.accountId, 'selected');
      assert.equal(context.env, env); made += 1;
      return { async close() { closed += 1; } };
    },
    createServer(_config, { provider }) { return { close: () => provider.close() }; },
  });
  assert.equal(result, 0); assert.equal(made, 1); assert.equal(closed, 1);
});

function outputProcess() {
  const events = new EventEmitter();
  let output = '';
  events.stdout = { write(text) { output += text; } };
  events.stderr = events.stdout;
  return { events, read: () => output };
}

test('help exits before configuration or provider creation', async () => {
  const { events, read } = outputProcess();
  assert.equal(await run(['--help'], { process: events,
    get env() { throw new Error('must not read environment'); },
    createProvider() { throw new Error('must not create a provider'); },
  }), 0);
  assert.match(read(), /Usage: framerize-server/);
});

for (const failListen of [false, true]) {
  test(`CLI removes signal handlers after ${failListen ? 'listen failure' : 'shutdown'}`, async () => {
    const { events } = outputProcess();
    const records = [];
    let closeCalls = 0;
    let closing;
    const pending = run(['serve', '--port', '0'], { env: {}, process: events,
      logger: { info(event, fields) {
        records.push({ event, fields });
        if (event === 'server.listening') { events.emit('SIGINT'); events.emit('SIGTERM'); }
      } },
      createServer: () => ({
        async listen() {
          if (failListen) throw new Error('synthetic listen failure');
          return { port: 12345 };
        },
        close() {
          if (!closing) { closeCalls += 1; closing = Promise.resolve(); }
          return closing;
        },
      }),
    });
    if (failListen) await assert.rejects(pending, /synthetic listen failure/);
    else { assert.equal(await pending, 0); assert.equal(records[0].fields.port, 12345); }
    assert.equal(closeCalls, 1);
    for (const signal of ['SIGINT', 'SIGTERM']) assert.equal(events.listenerCount(signal), 0);
  });
}

for (const signal of ['SIGINT', 'SIGTERM']) {
  test(`${signal} during server construction prevents listening`, async () => {
    const { events } = outputProcess();
    let closes = 0;
    const records = [];
    const result = await run(['serve', '--port', '0'], { env: {}, process: events,
      logger: { info(event) { records.push(event); } },
      async createServer() {
        events.emit(signal);
        return {
          async listen() { assert.fail('Cancelled startup must not listen'); },
          async close() { closes += 1; },
        };
      },
    });
    assert.equal(result, 0);
    assert.equal(closes, 1);
    assert.deepEqual(records, []);
    for (const name of ['SIGINT', 'SIGTERM']) assert.equal(events.listenerCount(name), 0);
  });
}

test('server construction failure closes the created provider', async () => {
  let closes = 0;
  await assert.rejects(run(['check', '--upstream', 'https://synthetic.invalid'], { env: {},
    createProvider: () => ({ close() { closes += 1; } }),
    createServer() { throw new Error('synthetic construction failure'); },
  }), /synthetic construction failure/);
  assert.equal(closes, 1);
});

test('startup cancellation waits for a late provider and closes it without constructing the server', async () => {
  const { events } = outputProcess();
  let release; let started; let signal; let settled = false; let closes = 0;
  const entered = new Promise(resolve => { started = resolve; });
  const pending = run(['serve', '--upstream', 'https://synthetic.invalid'], {
    env: {}, process: events,
    createProvider(config, context) {
      signal = context.signal; started();
      return new Promise(resolve => { release = () => resolve({ async close() { closes += 1; } }); });
    },
    createServer() { assert.fail('Cancelled provider startup must not construct the server'); },
  });
  pending.then(() => { settled = true; }, () => { settled = true; });
  await entered;
  events.emit('SIGTERM'); events.emit('SIGINT');
  assert.equal(signal.aborted, true);
  await Promise.resolve();
  assert.equal(settled, false);
  release();
  assert.equal(await pending, 0);
  assert.equal(closes, 1);
  for (const name of ['SIGINT', 'SIGTERM']) assert.equal(events.listenerCount(name), 0);
});

test('async provider creation transfers one cleanup owner to the server', async () => {
  const { events, read } = outputProcess();
  let closes = 0;
  const provider = { async close() { closes += 1; } };
  assert.equal(await run(['check', '--upstream', 'https://synthetic.invalid'], {
    env: {}, process: events,
    async createProvider(config, { signal }) {
      assert.equal(signal.aborted, false);
      return provider;
    },
    createServer(config, dependencies) {
      assert.equal(dependencies.provider, provider);
      return { async listen() { assert.fail('Check must not listen'); }, close: () => provider.close() };
    },
  }), 0);
  assert.equal(closes, 1);
  assert.match(read(), /config.valid/);
  for (const name of ['SIGINT', 'SIGTERM']) assert.equal(events.listenerCount(name), 0);
});

test('cancellation closes a pending listen and retains handlers throughout cleanup', async () => {
  const { events, read } = outputProcess();
  let rejectListen; let closes = 0;
  assert.equal(await run(['serve'], { env: {}, process: events,
    createServer: () => ({
      listen: () => new Promise((resolveUnused, reject) => {
        rejectListen = reject; events.emit('SIGTERM');
      }),
      async close() {
        closes += 1;
        for (const name of ['SIGINT', 'SIGTERM']) assert.equal(events.listenerCount(name), 1);
        events.emit('SIGINT'); events.emit('SIGTERM');
        rejectListen(new Error('Synthetic listen cancelled'));
      },
    }),
  }), 0);
  assert.equal(closes, 1);
  assert.equal(read(), '');
  for (const name of ['SIGINT', 'SIGTERM']) assert.equal(events.listenerCount(name), 0);
});

test('provider rejection after cancellation stops cleanly, but cleanup failure remains fatal', async () => {
  for (const closeFailure of [false, true]) {
    const { events } = outputProcess();
    const pending = run(['serve', '--upstream', 'https://synthetic.invalid'], {
      env: {}, process: events,
      async createProvider(config, { signal }) {
        events.emit('SIGTERM');
        if (!closeFailure) signal.throwIfAborted();
        return { async close() { throw new Error('Synthetic cleanup failure'); } };
      },
      createServer() { assert.fail('Cancelled startup must not construct the server'); },
    });
    if (closeFailure) await assert.rejects(pending, /Synthetic cleanup failure/);
    else assert.equal(await pending, 0);
    for (const name of ['SIGINT', 'SIGTERM']) assert.equal(events.listenerCount(name), 0);
  }
});

test('actual launcher reports constant fatal output without echoing bad input', context => {
  const root = mkdtempSync(join(tmpdir(), 'server-cli-fatal-'));
  context.after(() => rmSync(root, { recursive: true, force: true }));
  const launcher = fileURLToPath(new URL('../bin/framerize-server.js', import.meta.url));
  const result = spawnSync(process.execPath, [launcher, 'serve', '--port', 'synthetic-canary'], {
    cwd: root, encoding: 'utf8', timeout: 5000,
    env: { HOME: root, TMPDIR: root, NODE_DISABLE_COMPILE_CACHE: '1' },
  });
  assert.equal(result.error, undefined);
  assert.equal(result.status, 1);
  assert.equal(result.stdout, '');
  assert.deepEqual(JSON.parse(result.stderr), {
    event: 'server.failed', message: 'Server failed to start or stop',
  });
  assert.doesNotMatch(result.stderr, /synthetic-canary/);
});

test('CLI parsing preserves its input and maps supported options', () => {
  const args = ['serve', '--port', '0', '--server-token-file', '/private/synthetic'];
  const original = [...args];
  assert.deepEqual(parseArgs(args), { command: 'serve',
    values: { port: '0', serverTokenFile: '/private/synthetic' } });
  assert.deepEqual(args, original);
  assert.deepEqual(parseArgs([]), { command: 'serve', values: {} });
  for (const args of [['canary'], ['--canary', 'value'], ['--port'], ['--port', '--canary']]) {
    assert.throws(() => parseArgs(args), error => {
      assert.doesNotMatch(error.message, /canary/);
      return /Unknown command|Invalid option/.test(error.message);
    });
  }
});
