import test from 'node:test';
import assert from 'node:assert/strict';
import { parseArgs, run } from '../src/cli.js';
import { parseConfig } from '../src/config.js';
import { spawn, spawnSync } from 'node:child_process';
import { setTimeout as delay } from 'node:timers/promises';
import { EventEmitter } from 'node:events';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

function fixtureOptions(context) {
  const directory = mkdtempSync(path.join(tmpdir(), 'nexus-cli-test-'));
  context.after(() => rmSync(directory, { recursive: true, force: true }));
  const accounts = path.join(directory, 'accounts.json');
  writeFileSync(accounts, '[]');
  return ['--host', '127.0.0.1', '--port', '0', '--accounts', accounts,
    '--upstream', 'http://127.0.0.1:1'];
}

function fixtureEnvironment(environment = process.env) {
  return { PATH: environment.PATH, SystemRoot: environment.SystemRoot };
}

test('default CLI config preserves loopback and default port', () => {
  const config = parseConfig(parseArgs([], {}).values);
  assert.equal(config.host, '127.0.0.1');
  assert.equal(config.port, 8788);
});

test('CLI fixtures exclude poisoned host configuration', (context) => {
  const poisoned = {
    PATH: '/fixture/bin', SystemRoot: '/fixture/system',
    FRAMER_ACCOUNTS: '/fixture/private.json', FRAMER_HOST: '0.0.0.0',
    FRAMER_PORT: '-1', FRAMER_UPSTREAM: 'invalid',
    UPSTREAM_TOKEN: 'fixture-only', NODE_OPTIONS: '--require=/fixture/private.js',
  };
  assert.deepEqual(fixtureEnvironment(poisoned), {
    PATH: poisoned.PATH, SystemRoot: poisoned.SystemRoot,
  });
  const config = parseConfig(parseArgs(['check', ...fixtureOptions(context)], poisoned).values);
  assert.equal(config.host, '127.0.0.1');
  assert.equal(config.port, 0);
  assert.equal(config.upstream, 'http://127.0.0.1:1');
  assert.equal(readFileSync(config.accounts, 'utf8'), '[]');
});

test('check validates without listening', async (context) => {
  assert.equal(await run(['check', ...fixtureOptions(context)]), 0);
});

test('shutdown failures set a nonzero exit code', async (context) => {
  const events = new EventEmitter();
  let close;
  const running = run(['serve', ...fixtureOptions(context)], { process: events, createServer: async () => ({
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
  const result = spawnSync(process.execPath, ['bin/framerize-nexus.js', 'serve', '--port', '-1'], {
    encoding: 'utf8', env: fixtureEnvironment(),
  });
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /positive integer/);
});

test('CLI help exits successfully', () => {
  const result = spawnSync(process.execPath, ['bin/framerize-nexus.js', '--help'], {
    encoding: 'utf8', env: fixtureEnvironment(),
  });
  assert.equal(result.status, 0);
  assert.match(result.stdout, /Usage:/);
});

test('CLI serve shuts down once on SIGTERM', { skip: process.platform === 'win32' }, async () => {
  const child = spawn(process.execPath, ['bin/framerize-nexus.js', 'serve', '--port', '0'], {
    stdio: ['ignore', 'pipe', 'pipe'], env: fixtureEnvironment(),
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

test('CLI shares shutdown across SIGINT and SIGTERM', { skip: process.platform === 'win32' }, async () => {
  const child = spawn(process.execPath, ['bin/framerize-nexus.js', 'serve', '--port', '0'], {
    stdio: ['ignore', 'pipe', 'pipe'], env: fixtureEnvironment(),
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
