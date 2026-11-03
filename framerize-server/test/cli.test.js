import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs, run } from '../dist/cli.js';

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
      logger: { info(event, fields) { records.push({ event, fields }); } },
      createServer: () => ({
        async listen() {
          if (failListen) throw new Error('synthetic listen failure');
          queueMicrotask(() => { events.emit('SIGINT'); events.emit('SIGTERM'); });
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

test('server construction failure closes the created provider', async () => {
  let closes = 0;
  await assert.rejects(run(['check', '--upstream', 'https://synthetic.invalid'], { env: {},
    createProvider: () => ({ close() { closes += 1; } }),
    createServer() { throw new Error('synthetic construction failure'); },
  }), /synthetic construction failure/);
  assert.equal(closes, 1);
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
