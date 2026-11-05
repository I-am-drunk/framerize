import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter, once } from 'node:events';
import { spawn } from 'node:child_process';
import { createConnection } from 'node:net';
import { setTimeout as delay } from 'node:timers/promises';
import { PassThrough } from 'node:stream';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/compatibility/process.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
const { nodeRunner } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const command = { command: 'synthetic-client', args: ['--version'], display: 'synthetic-client --version' };

async function assertPortCloses(port) {
  for (let attempt = 0; attempt < 40; attempt++) {
    const code = await new Promise(resolve => {
      const socket = createConnection({ host: '127.0.0.1', port });
      socket.once('connect', () => { socket.destroy(); resolve('open'); });
      socket.once('error', error => { socket.destroy(); resolve(error.code); });
    });
    if (code === 'ECONNREFUSED') return;
    await delay(20);
  }
  assert.fail('the captured descendant kept its listener open after the capture deadline');
}

function fixture(t, platform = 'linux') {
  t.mock.timers.enable({ apis: ['setTimeout'] });
  const child = new EventEmitter();
  child.stdout = new PassThrough();
  child.kills = [];
  child.kill = signal => { child.kills.push(signal); return true; };
  child.unref = () => {};
  const calls = [];
  const runner = nodeRunner(platform, (...args) => { calls.push(args); return child; });
  t.after(() => { child.emit('close', null, 'SIGKILL'); child.stdout.destroy(); });
  return { child, runner, calls };
}

test('capture returns null at the deadline even if the child never closes stdout', async t => {
  const { child, runner } = fixture(t);
  let result = 'pending';
  runner.capture(command, {}).then(value => { result = value; });
  t.mock.timers.tick(5000);
  await Promise.resolve();
  assert.equal(result, null);
  assert.deepEqual(child.kills, ['SIGKILL']);
});

test('capture rejects oversized output without waiting for close', async t => {
  const { child, runner } = fixture(t);
  let result = 'pending';
  runner.capture(command, {}).then(value => { result = value; });
  child.stdout.write(Buffer.alloc(65537));
  await Promise.resolve();
  assert.equal(result, null);
  assert.equal(child.stdout.destroyed, true);
});

test('capture handles a spawn error without waiting for close', async t => {
  const { child, runner } = fixture(t);
  let result = 'pending';
  runner.capture(command, {}).then(value => { result = value; });
  child.emit('error', new Error('synthetic launch failure'));
  await Promise.resolve();
  assert.equal(result, null);
});

test('capture preserves output through a normal successful close', async t => {
  const { child, runner } = fixture(t);
  const result = runner.capture(command, {});
  child.stdout.write('1.2.');
  child.stdout.write('3\n');
  child.emit('close', 0);
  assert.equal(await result, '1.2.3\n');
  t.mock.timers.tick(5000);
  assert.deepEqual(child.kills, []);
});

test('Windows timeout terminates the captured process tree', async t => {
  const { child, runner, calls } = fixture(t, 'win32');
  child.pid = 987654;
  let result = 'pending';
  runner.capture({ ...command, command: 'synthetic-client.cmd' }, {}).then(value => { result = value; });
  t.mock.timers.tick(5000);
  await Promise.resolve();
  assert.equal(result, null);
  assert.equal(calls.length, 2);
  assert.equal(calls[1][0], 'taskkill');
  assert.deepEqual(calls[1][1], ['/PID', '987654', '/T', '/F']);
  assert.equal(calls[1][2].shell, false);
});

test('a timed-out POSIX probe closes its descendant listener after the direct child exits',
  { skip: process.platform === 'win32', timeout: 9000 }, async t => {
    const descendant = `const net = require('node:net');
      const server = net.createServer(socket => socket.end());
      server.listen(0, '127.0.0.1', () => {
        console.log(JSON.stringify({ port: server.address().port, pid: process.pid }));
        process.send('ready');
      });`;
    const parent = `const child = require('node:child_process').spawn(process.execPath,
      ['-e', ${JSON.stringify(descendant)}], { stdio: ['ignore', 'inherit', 'ignore', 'ipc'] });
      child.once('message', () => process.exit(0));`;
    let child;
    const runner = nodeRunner(process.platform, (...args) => { child = spawn(...args); return child; });
    const result = runner.capture({ command: process.execPath, args: ['-e', parent], display: 'synthetic probe' }, {});
    t.after(() => { try { process.kill(-child.pid, 'SIGKILL'); } catch {} child.stdout.destroy(); });
    const exited = once(child, 'exit');
    const [line] = await once(child.stdout, 'data');
    const { port } = JSON.parse(line.toString());
    await exited;
    assert.equal(await result, null);
    await assertPortCloses(port);
  });
