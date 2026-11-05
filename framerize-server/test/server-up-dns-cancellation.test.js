import assert from 'node:assert/strict';
import dns from 'node:dns';
import { once } from 'node:events';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

const packageRoot = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
const { runSetupCommand } = await import(pathToFileURL(join(packageRoot, 'dist/setup.js')).href);
const { renderServiceUnit } = await import(pathToFileURL(join(packageRoot, 'dist/service-units.js')).href);

async function bounded(promise, message) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), 5000);
    })]);
  } finally { clearTimeout(timer); }
}

async function refused(port) {
  const socket = net.createConnection({ host: '127.0.0.1', port });
  try {
    return await bounded(new Promise(resolve => {
      socket.once('connect', () => resolve('CONNECTED'));
      socket.once('error', error => resolve(error.code));
    }), 'Port refusal check timed out');
  } finally { socket.destroy(); }
}

test('up restores the prior service when cancelled during DNS preflight', { timeout: 20_000 }, async t => {
  const root = await mkdtemp(join(tmpdir(), 'server-up-dns-cancel-'));
  const controller = new AbortController();
  const reason = new Error('synthetic DNS preflight cancellation');
  const servers = new Set(), sockets = new Set(), held = [];
  let outcome, released = false, port;
  const releaseDns = () => {
    released = true;
    for (const reply of held.splice(0)) reply();
  };
  t.after(async () => {
    controller.abort(reason);
    releaseDns();
    try {
      if (outcome) await bounded(outcome, 'Setup did not settle during cleanup');
    } finally {
      await new Promise(setImmediate);
      for (const socket of sockets) socket.destroy();
      try {
        await bounded(Promise.all([...servers].map(server => new Promise(resolve => {
          server.close(() => resolve());
        }))), 'Owned listener cleanup timed out');
        if (port) assert.equal(await refused(port), 'ECONNREFUSED');
      } finally { await rm(root, { recursive: true, force: true }); }
      t.diagnostic(JSON.stringify({ phase: 'cleanup', privateStateRemoved: true,
        ownedListenersOpen: [...servers].filter(server => server.listening).length,
        sockets: sockets.size, portRefused: Boolean(port) }));
    }
  });
  const selector = net.createServer();
  servers.add(selector);
  selector.listen(0, '127.0.0.1');
  await bounded(once(selector, 'listening'), 'Port allocation timed out');
  port = selector.address().port;
  await new Promise(resolve => selector.close(resolve));
  const home = join(root, 'server'), xdg = join(root, 'xdg');
  const settingsPath = join(home, 'server.json');
  const unitPath = join(xdg, 'systemd', 'user', 'framerize-server.service');
  const priorSettings = Buffer.from(`${JSON.stringify({ version: 1, config: {
    backend: 'http', host: '127.0.0.1', port, upstream: 'http://127.0.0.1:1',
  }, environment: {} })}\n`);
  const priorUnit = Buffer.from(renderServiceUnit({ home, homeDir: root, platform: 'linux',
    uid: 1000, xdgConfigHome: xdg, environmentPath: '',
    program: [process.execPath, join(root, 'synthetic-prior-server.js')] }));
  await mkdir(home, { recursive: true });
  await mkdir(dirname(unitPath), { recursive: true });
  await writeFile(settingsPath, priorSettings, { mode: 0o600 });
  await writeFile(unitPath, priorUnit, { mode: 0o600 });
  const state = { running: true, enabled: true };
  const calls = [];
  const run = async (command, args, signal) => {
    calls.push([command, ...args]);
    assert.ok(!signal?.aborted, 'Rollback must not inherit an aborted signal');
    if (command === 'loginctl') {
      assert.deepEqual(args, ['show-user', '1000', '--property=Linger', '--value']);
      return { code: 0, stdout: 'yes' };
    }
    assert.equal(command, 'systemctl');
    assert.equal(args[0], '--user');
    const action = args[1];
    if (action === 'show') return { code: 0, stdout: state.running
      ? 'ActiveState=active\nSubState=running\nMainPID=41001\n'
      : 'ActiveState=inactive\nSubState=dead\nMainPID=0\n' };
    if (action === 'is-enabled') return { code: state.enabled ? 0 : 1, stdout: '' };
    assert.ok(['show-environment', 'stop', 'disable', 'enable', 'restart', 'daemon-reload'].includes(action));
    if (action === 'stop' || action === 'disable') state.running = false;
    if (action === 'disable') state.enabled = false;
    if (action === 'enable') state.enabled = true;
    if (action === 'restart') {
      assert.deepEqual(await readFile(settingsPath), priorSettings, 'Only restore the prior settings');
      assert.deepEqual(await readFile(unitPath), priorUnit, 'Only restore the prior unit');
      state.running = true;
    }
    return { code: 0, stdout: '' };
  };
  const syntheticHost = 'preflight-only.invalid';
  const probes = new Set(), listening = [];
  let notifyLookup, lookups = 0;
  const lookupPending = new Promise(resolve => { notifyLookup = resolve; });
  t.mock.method(dns, 'lookup', (hostname, options, callback) => {
    assert.equal(hostname, syntheticHost, 'No real DNS lookup is permitted');
    lookups++;
    const done = typeof options === 'function' ? options : callback;
    const reply = () => options?.all
      ? done(null, [{ address: '127.0.0.1', family: 4 }]) : done(null, '127.0.0.1', 4);
    if (released) setImmediate(reply);
    else held.push(reply);
    notifyLookup();
  });
  const originalListen = net.Server.prototype.listen;
  t.mock.method(net.Server.prototype, 'listen', function (...args) {
    if (args[0]?.host === syntheticHost) {
      assert.equal(args[0].port, port);
      probes.add(this);
      servers.add(this);
      this.on('listening', () => listening.push({ released }));
      this.on('connection', socket => {
        sockets.add(socket);
        socket.once('close', () => sockets.delete(socket));
      });
    }
    return Reflect.apply(originalListen, this, args);
  });
  let output = '', readyCalls = 0;
  outcome = runSetupCommand(['up', '--backend', 'http', '--upstream', 'http://127.0.0.1:2',
    '--host', syntheticHost, '--port', String(port), '--no-lan', '--no-tailscale', '--json'], {
    signal: controller.signal, env: { HOME: root, XDG_CONFIG_HOME: xdg, FRAMERIZE_SERVER_HOME: home },
    platform: 'linux', uid: 1000, program: [process.execPath, join(packageRoot, 'bin/framerize-server.js')],
    run, stdout: { write: text => { output += text; } },
    ready: async () => { readyCalls++; assert.fail('Cancelled setup must not request readiness'); },
  }).then(value => ({ status: 'fulfilled', value }), error => ({ status: 'rejected', error }));
  await bounded(Promise.race([lookupPending, outcome.then(result => {
    throw new Error(`Setup settled before DNS was held: ${result.status}`, { cause: result.error });
  })]), 'Synthetic DNS lookup was not reached');
  assert.equal(state.running, false, 'The prior service must be stopped before cancellation');
  assert.ok(calls.some(args => args[2] === 'stop'));
  controller.abort(reason);
  const cancelled = await bounded(outcome, 'Cancellation did not settle while DNS was held')
    .catch(error => ({ status: 'pending', error }));
  const beforeRelease = { status: cancelled.status, sameReason: cancelled.error === reason,
    ...state, settingsRestored: (await readFile(settingsPath)).equals(priorSettings),
    unitRestored: (await readFile(unitPath)).equals(priorUnit), readyCalls,
    restarts: calls.filter(args => args[2] === 'restart').length, output };
  const callsBeforeRelease = structuredClone(calls);
  t.diagnostic(JSON.stringify({ phase: 'before DNS release', ...beforeRelease }));
  releaseDns();
  const settled = await bounded(outcome, 'Setup did not settle after DNS release');
  await new Promise(setImmediate);
  const portResult = await refused(port);
  const lateActivations = calls.slice(callsBeforeRelease.length)
    .filter(args => ['enable', 'restart'].includes(args[2]));
  t.diagnostic(JSON.stringify({ phase: 'after DNS release', status: settled.status,
    sameReason: settled.error === reason, ...state, lateActivations,
    listening, activeProbes: [...probes].filter(probe => probe.listening).length,
    portResult, readyCalls, output, lookups }));
  assert.equal(cancelled.status, 'rejected', 'Cancellation must reject before DNS is released');
  assert.equal(cancelled.error, reason);
  assert.equal(beforeRelease.running, true, 'Restore the prior service before releasing DNS');
  assert.equal(beforeRelease.enabled, true);
  assert.equal(beforeRelease.settingsRestored, true);
  assert.equal(beforeRelease.unitRestored, true);
  assert.equal(settled.status, 'rejected');
  assert.equal(settled.error, reason);
  assert.deepEqual(state, { running: true, enabled: true });
  assert.deepEqual(await readFile(settingsPath), priorSettings);
  assert.deepEqual(await readFile(unitPath), priorUnit);
  assert.deepEqual(lateActivations, [], 'No activation may occur after DNS is released');
  assert.deepEqual(listening, [], 'Cancelled preflight must never open a listener');
  assert.ok([...probes].every(probe => !probe.listening));
  assert.equal(portResult, 'ECONNREFUSED');
  assert.equal(readyCalls, 0);
  assert.equal(output, '');
  assert.equal(held.length, 0);
  assert.equal(lookups, 1);
});
