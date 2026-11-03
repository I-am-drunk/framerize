import test from 'node:test';
import assert from 'node:assert/strict';
import { parseArgs, run } from '../src/cli.js';
import { parseConfig } from '../src/config.js';
import { spawn, spawnSync } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';
import { EventEmitter } from 'node:events';

test('default CLI config preserves loopback and default port', () => {
  const config = parseConfig(parseArgs([], {}).values);
  assert.equal(config.host, '127.0.0.1');
  assert.equal(config.port, 8788);
});

test('check validates without listening', async () => {
  assert.equal(await run(['check']), 0);
});

test('shutdown failures set a nonzero exit code', async () => {
  const events = new EventEmitter();
  let close;
  const running = run(['serve', '--port', '0'], { process: events, createServer: async () => ({
    server: { once(event, callback) { if (event === 'close') close = callback; } },
    listen: async () => ({ port: 1234 }),
    close: async () => { throw new Error('fixture close failure'); },
  }) });
  await new Promise((resolve) => setImmediate(resolve));
  events.emit('SIGTERM');
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(events.exitCode, 1);
  close();
  assert.equal(await running, 0);
});

test('unknown options fail', () => {
  assert.throws(() => parseArgs(['serve', '--unknown']), /unknown option/);
});

test('installed CLI rejects invalid options without listening', () => {
  const result = spawnSync(process.execPath, ['bin/framer-next.js', 'serve', '--port', '-1'], {
    encoding: 'utf8', env: { PATH: process.env.PATH },
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /positive integer/);
});

test('CLI help exits successfully', () => {
  const result = spawnSync(process.execPath, ['bin/framer-next.js', '--help'], { encoding: 'utf8' });
  assert.equal(result.status, 0);
  assert.match(result.stdout, /Usage:/);
});

test('CLI serve shuts down once on SIGTERM', async () => {
  const child = spawn(process.execPath, ['bin/framer-next.js', 'serve', '--port', '0'], {
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let logs = '';
  child.stderr.on('data', (chunk) => { logs += chunk; });
  try {
    await new Promise((resolve, reject) => {
      if (logs.includes('server.listening')) resolve();
      child.stderr.on('data', () => { if (logs.includes('server.listening')) resolve(); });
      child.once('exit', (code) => reject(new Error(`server exited ${code}`)));
      setTimeout(() => reject(new Error('readiness timeout')), 5000).unref();
    });
    child.kill('SIGTERM');
    const result = await Promise.race([
      new Promise((resolve) => child.once('exit', (code, signal) => resolve({ code, signal }))),
      delay(5000).then(() => { throw new Error('shutdown timeout'); }),
    ]);
    assert.ok(result.code === 0 || result.signal === 'SIGTERM', JSON.stringify(result));
    assert.equal((logs.match(/server.closed/g) || []).length, 1);
  } finally { child.kill('SIGKILL'); }
});

test('CLI shares shutdown across SIGINT and SIGTERM', async () => {
  const child = spawn(process.execPath, ['bin/framer-next.js', 'serve', '--port', '0'], {
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let logs = '';
  child.stderr.on('data', (chunk) => { logs += chunk; });
  try {
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('readiness timeout')), 5000);
      const exited = () => { clearTimeout(timer); reject(new Error('server exited')); };
      child.once('exit', exited);
      child.stderr.on('data', () => {
        if (!logs.includes('server.listening')) return;
        clearTimeout(timer);
        child.off('exit', exited);
        resolve();
      });
    });
    child.kill('SIGINT');
    child.kill('SIGTERM');
    const result = await Promise.race([
      new Promise((resolve) => child.once('exit', (code, signal) => resolve({ code, signal }))),
      delay(5000).then(() => { throw new Error('shutdown timeout'); }),
    ]);
    assert.ok(result.code === 0 || result.signal === 'SIGTERM', JSON.stringify(result));
    assert.equal((logs.match(/server.closed/g) || []).length, 1);
  } finally { child.kill('SIGKILL'); }
});
