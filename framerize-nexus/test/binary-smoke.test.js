import test from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter, once } from 'node:events';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';

const source = readFileSync(new URL('../scripts/binary-smoke.js', import.meta.url), 'utf8');
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;
function loadScript(text) {
  return new AsyncFunction('assert', 'execFileSync', 'spawn', 'existsSync',
    'mkdtempSync', 'readFileSync', 'rmSync', 'once', 'tmpdir', 'path', 'http',
    'process', 'fetch', 'console', 'setTimeout', 'clearTimeout', text.replace(/^import .*;\r?\n/gm, ''));
}
const execute = loadScript(source);

async function fixture(context, platform, failure = '', archive = true, run = execute) {
  const root = mkdtempSync(path.join(tmpdir(), 'nexus-cleanup-test-'));
  context.after(() => rmSync(root, { recursive: true, force: true }));
  const retained = path.join(root, 'pre-existing.txt');
  const input = path.join(root, 'archive.fixture');
  writeFileSync(retained, 'retain');
  writeFileSync(input, 'original archive');
  const events = [];
  const removed = [];
  let allocated;
  const writePayload = (directory) => {
    const name = platform === 'win32' ? 'framerize-nexus.exe' : 'framerize-nexus';
    writeFileSync(path.join(directory, name), 'executable');
    writeFileSync(path.join(directory, 'LICENSE'), 'project notice');
    writeFileSync(path.join(directory, 'NODE-LICENSE'), 'Node notice');
  };
  const dist = path.join(root, 'dist');
  mkdirSync(dist);
  writePayload(dist);
  const upstream = new EventEmitter();
  upstream.listen = () => queueMicrotask(() => upstream.emit('listening'));
  upstream.address = () => ({ port: 1234 });
  upstream.closeAllConnections = () => events.push('sockets closed');
  upstream.close = (callback) => queueMicrotask(() => {
    events.push('upstream closed');
    callback();
  });
  const child = new EventEmitter();
  child.stderr = new EventEmitter();
  child.pid = failure === 'spawn' ? undefined : 42;
  child.exitCode = null;
  child.signalCode = null;
  child.kill = (signal) => {
    events.push(signal);
    if (failure !== 'child-timeout') queueMicrotask(() => {
      child.signalCode = signal;
      child.emit('exit', null, signal);
      events.push('child closed');
      child.emit('close', null, signal);
    });
    return true;
  };
  const runCommand = (command, args) => {
    if (command === 'tar' || command === 'powershell') {
      writePayload(allocated);
      if (failure === 'extraction') throw new Error('extraction failure');
      return '';
    }
    if (failure === 'help' && args[0] === '--help') throw new Error('help failure');
    if (failure === 'check' && args[0] === 'check') throw new Error('check failure');
    return args[0] === '--help' ? 'Framerize Nexus' : '';
  };
  const spawn = () => {
    queueMicrotask(() => {
      if (failure === 'spawn') {
        child.emit('error', new Error('spawn failure'));
        events.push('child closed');
        child.emit('close', -1);
      } else if (failure !== 'startup-timeout') {
        child.stderr.emit('data', Buffer.from('{"port":5678}'));
      }
    });
    return child;
  };
  const timers = new Set();
  const setTimer = (callback, delay) => {
    assert.equal(delay, 5000);
    const timer = { active: true };
    timers.add(timer);
    if (failure.endsWith('timeout')) queueMicrotask(() => { if (timer.active) callback(); });
    return timer;
  };
  const clearTimer = (timer) => { if (timer) timer.active = false; };
  let error;
  try {
    await run(assert, runCommand, spawn, existsSync, (prefix) => {
      allocated = mkdtempSync(prefix);
      return allocated;
    }, (file, ...args) => {
      if (failure === 'license' && file.endsWith('NODE-LICENSE')) throw new Error('license failure');
      return readFileSync(file, ...args);
    }, (directory, options) => {
      events.push('directory removed');
      removed.push(directory);
      rmSync(directory, options);
    }, once, () => root, { ...path, resolve: (...parts) => path.resolve(root, ...parts) },
    { createServer: () => upstream },
    { platform, argv: ['node', 'binary-smoke.js', ...(archive ? [input] : [])],
      env: { SystemRoot: 'C:\\Windows', FRAMER_ACCOUNTS: '/never-read' } },
    async () => {
      if (failure === 'fetch') throw new Error('fetch failure');
      return { text: async () => 'data: fixture\n\n' };
    }, { log() {} }, setTimer, clearTimer);
  } catch (caught) {
    error = caught;
  }
  assert.deepEqual(removed, [allocated]);
  assert.equal(existsSync(allocated), false);
  assert.equal(readFileSync(retained, 'utf8'), 'retain');
  assert.equal(readFileSync(input, 'utf8'), 'original archive');
  assert.equal(readFileSync(path.join(dist, 'LICENSE'), 'utf8'), 'project notice');
  assert.ok([...timers].every((timer) => !timer.active));
  if (events.includes('child closed')) {
    assert.ok(events.indexOf('child closed') < events.indexOf('directory removed'));
  }
  if (events.includes('upstream closed')) {
    assert.ok(events.indexOf('sockets closed') < events.indexOf('upstream closed'));
    assert.ok(events.indexOf('upstream closed') < events.indexOf('directory removed'));
  }
  return { error, events };
}

test('loads the smoke script from a CRLF checkout', async (context) => {
  const result = await fixture(context, 'win32', '', true, loadScript(source.replace(/\r?\n/g, '\r\n')));
  assert.equal(result.error, undefined);
});

for (const platform of ['linux', 'darwin', 'win32']) {
  for (const failure of ['', 'extraction', 'license', 'help', 'check', 'spawn', 'startup-timeout', 'fetch', 'child-timeout']) {
    test(`${platform}: removes only the allocated directory after ${failure || 'success'}`, async (context) => {
      const result = await fixture(context, platform, failure);
      if (failure) {
        const message = failure === 'startup-timeout' ? 'readiness timeout'
          : failure === 'child-timeout' ? 'child cleanup timeout' : `${failure} failure`;
        assert.equal(result.error?.message, message);
      } else {
        assert.equal(result.error, undefined);
        assert.deepEqual(result.events, ['SIGTERM', 'child closed', 'sockets closed',
          'upstream closed', 'directory removed']);
      }
    });
  }
  test(`${platform}: leaves checkout dist files untouched without an archive argument`, async (context) => {
    const result = await fixture(context, platform, '', false);
    assert.equal(result.error, undefined);
  });
}
