import test from 'node:test';
import assert from 'node:assert/strict';
import { parseArgs, run } from '../src/cli.js';
import { parseConfig } from '../src/config.js';
import { spawn, spawnSync } from 'node:child_process';

test('default CLI config preserves loopback and default port', () => {
  const config = parseConfig(parseArgs([], {}).values);
  assert.equal(config.host, '127.0.0.1');
  assert.equal(config.port, 8788);
});

test('check validates without listening', async () => {
  assert.equal(await run(['check']), 0);
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
    assert.equal(await new Promise((resolve) => {
      if (child.exitCode !== null) resolve(child.exitCode);
      else child.once('exit', resolve);
    }), 0);
    assert.equal((logs.match(/server.closed/g) || []).length, 1);
  } finally { child.kill('SIGKILL'); }
});
