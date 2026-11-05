import assert from 'node:assert/strict';
import test from 'node:test';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runSetupCommand } from '../dist/setup.js';
import { captureSettings } from '../dist/settings.js';
import { loadConfig } from '../dist/config.js';

async function fixture(t, paired = true) {
  const root = await mkdtemp(join(tmpdir(), 'server-setup-routes-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const home = join(root, 'server');
  const path = join(home, 'server.json');
  const port = await freePort();
  const target = 'http://127.0.0.1:' + port;
  const config = loadConfig({ backend: 'http', upstream: 'http://127.0.0.1:1',
    host: '127.0.0.1', port }, {});
  const saved = { ...captureSettings(config, {}),
    ...(paired ? { access: { tailscalePort: 443 } } : {}) };
  await mkdir(home, { recursive: true });
  await writeFile(path, JSON.stringify(saved), { mode: 0o600 });
  const state = { routes: new Map(paired ? [[443, target]] : []),
    running: false, enabled: false, calls: [], output: '' };
  const run = async (command, args) => {
    state.calls.push([command, ...args]);
    if (command === 'loginctl') return { code: 0, stdout: 'yes' };
    if (command === 'systemctl') {
      if (args.includes('is-enabled')) return { code: state.enabled ? 0 : 1, stdout: '' };
      if (args.includes('is-active')) return { code: state.running ? 0 : 1, stdout: '' };
      if (args.includes('show')) return { code: 0, stdout: state.running
        ? 'ActiveState=active\nSubState=running\nMainPID=1234\n'
        : 'ActiveState=inactive\nSubState=dead\nMainPID=0\n' };
      if (args.includes('enable')) state.enabled = true;
      if (args.includes('restart')) state.running = true;
      if (args.includes('stop') || args.includes('disable')) state.running = false;
      if (args.includes('disable')) state.enabled = false;
      return { code: 0, stdout: '' };
    }
    assert.match(command, /^tailscale(?:\.exe)?$/);
    if (args[0] === 'status') return { code: 0, stdout: JSON.stringify({
      Self: { DNSName: 'fixture.example.ts.net.', CapMap: { https: [] } },
    }) };
    if (args[1] === 'status') return { code: 0, stdout: JSON.stringify({
      Web: Object.fromEntries([...state.routes].map(([port, Proxy]) =>
        ['fixture.example.ts.net:' + port, { Handlers: { '/': { Proxy } } }])),
    }) };
    const port = Number(args.find(arg => arg.startsWith('--https=')).slice(8));
    if (args.includes('--bg')) state.routes.set(port, args.at(-1));
    if (args.at(-1) === 'off') state.routes.delete(port);
    return { code: 0, stdout: '' };
  };
  const deps = { env: { HOME: root, FRAMERIZE_SERVER_HOME: home },
    platform: 'linux', uid: 1000, program: ['/synthetic/node', '/synthetic/server'],
    stdout: { write(text) { state.output += text; } }, run, ready: async () => 1234 };
  return { root, home, path, port, target, saved, state, deps };
}

async function freePort() {
  const listener = createServer();
  await new Promise((resolve, reject) => {
    listener.once('error', reject);
    listener.listen(0, '127.0.0.1', resolve);
  });
  const port = listener.address().port;
  await new Promise((resolve, reject) => listener.close(error => error ? reject(error) : resolve()));
  return port;
}

test('up moves its saved route when the local port changes', async t => {
  const { deps, state, path } = await fixture(t);
  const port = await freePort();
  await runSetupCommand(['up', '--port', String(port), '--json'], deps);
  assert.equal(state.routes.get(443), 'http://127.0.0.1:' + port);
  assert.equal(JSON.parse(await readFile(path, 'utf8')).config.port, port);
});

test('up removes its previous HTTPS route when the Serve port changes', async t => {
  const { deps, state, target } = await fixture(t);
  await runSetupCommand(['up', '--tailscale-serve-port', '8443', '--json'], deps);
  assert.equal(state.routes.has(443), false);
  assert.equal(state.routes.get(8443), target);
});

test('uninstall removes the owned route and saved access but keeps configuration', async t => {
  const { deps, state, path, root, port } = await fixture(t);
  await runSetupCommand(['up', '--json'], deps);
  await runSetupCommand(['service', 'uninstall', '--json'], deps);
  assert.equal(state.running, false);
  assert.equal(state.routes.size, 0);
  const saved = JSON.parse(await readFile(path, 'utf8'));
  assert.equal(saved.access, undefined);
  assert.equal(saved.config.port, port);
  await assert.rejects(readFile(join(root, '.config/systemd/user/framerize-server.service')),
    { code: 'ENOENT' });
});

test('pair cannot create a Tailscale route without saved ownership', async t => {
  const { deps, state, path } = await fixture(t, false);
  const before = await readFile(path, 'utf8');
  await assert.rejects(runSetupCommand(['pair', '--tailscale', '--json'], deps),
    /up --tailscale first/);
  assert.equal(state.output, '');
  assert.equal(state.routes.size, 0);
  assert.equal(await readFile(path, 'utf8'), before);
  assert.ok(state.calls.every(args => !args.includes('--bg')));
});

test('uninstall preserves a route replaced by another application', async t => {
  const { deps, state, path } = await fixture(t);
  await runSetupCommand(['up', '--json'], deps);
  const before = await readFile(path, 'utf8');
  state.routes.set(443, 'http://127.0.0.1:9999');
  await assert.rejects(runSetupCommand(['service', 'uninstall'], deps), /another application/);
  assert.equal(state.routes.get(443), 'http://127.0.0.1:9999');
  assert.equal(state.running, true);
  assert.equal(await readFile(path, 'utf8'), before);
});

test('route rollback failure retains the original readiness error', async t => {
  const { deps } = await fixture(t, false);
  const run = deps.run;
  deps.run = (command, args) => args.at(-1) === 'off'
    ? Promise.resolve({ code: 1, stdout: '' }) : run(command, args);
  deps.ready = async origin => {
    if (origin.startsWith('https:')) throw new Error('original route readiness failure');
    return 1234;
  };
  await assert.rejects(runSetupCommand(['up', '--tailscale', '--json'], deps), error => {
    assert.match(error.message, /original route readiness failure/);
    assert.match(error.message, /Rollback also failed/);
    return true;
  });
});
