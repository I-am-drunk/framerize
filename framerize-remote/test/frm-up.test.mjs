// The restart and rollback cases adapt T3 Code apps/server/src/cli/up.test.ts at 5a906583 (MIT, T3 Tools Inc.).
import test from 'node:test';
import assert from 'node:assert/strict';
import dns from 'node:dns';
import { syncBuiltinESMExports } from 'node:module';
import { createServer } from 'node:http';
import { mkdir, mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/cmdUp.ts', import.meta.url))],
  alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) },
  bundle: true, platform: 'node', format: 'esm', write: false });
const { setUp, formatUp } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const quiet = { stdout() {}, stderr() {} };

async function availablePort() {
  const probe = createServer();
  await new Promise(resolve => probe.listen(0, '127.0.0.1', resolve));
  const { port } = probe.address();
  await new Promise(resolve => probe.close(resolve));
  return port;
}

/** Stands in for the unit's `frm serve`: answers /health with the PID the manager reports. */
async function managedListener(control, port) {
  await closeManaged(control);
  if (!control.listens) return;
  const server = createServer((request, response) => {
    response.writeHead(200, { 'x-framerize-remote-pid': String(control.pid) });
    response.end('{"status":"ok"}');
  });
  await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); })
    .then(() => { control.managed = server; }, () => {});
}

async function closeManaged(control) {
  const server = control.managed;
  control.managed = undefined;
  if (server) await new Promise(resolve => { server.closeAllConnections(); server.close(resolve); });
}

async function fixture(context, { requireToken = 'synthetic-pair+/=', ready = true } = {}) {
  const root = await mkdtemp(join(tmpdir(), 'remote-frm-up-'));
  const seen = [];
  const remote = createServer((request, response) => {
    seen.push(request.headers.authorization);
    request.resume();
    const authorized = !requireToken || request.headers.authorization === `Bearer ${requireToken}`;
    response.writeHead(!authorized ? 401 : ready ? 200 : 503, { 'content-type': 'application/json' });
    response.end(JSON.stringify(ready ? { apiVersion: 1, status: 'ready',
      capabilities: { responses: true, chatCompletions: true, streaming: true } }
      : { apiVersion: 1, status: 'unavailable', reason: 'upstream_unavailable',
        capabilities: { responses: true, chatCompletions: true, streaming: true } }));
  });
  await new Promise(resolve => remote.listen(0, '127.0.0.1', resolve));
  context.after(async () => {
    await new Promise(resolve => remote.close(resolve));
    await rm(root, { recursive: true, force: true });
  });
  const commands = [];
  const port = await availablePort();
  // The fake unit runs a real local listener that reports its PID, as `frm serve` does.
  const control = { active: false, fail: undefined, failOnce: undefined, pid: 424242, listens: true, managed: undefined };
  const run = async (command, args) => {
    const line = `${command} ${args.join(' ')}`;
    commands.push(line);
    const failed = line === control.fail || line === control.failOnce;
    if (line === control.failOnce) control.failOnce = undefined;
    if (failed) return { code: 1, stdout: '' };
    if (/ (re)?start frm\.service$/.test(line)) { control.active = true; await managedListener(control, port); }
    if (/ stop frm\.service$| disable --now frm\.service$/.test(line)) { control.active = false; await closeManaged(control); }
    if (line.includes(' is-active ') && !control.active) return { code: 3, stdout: '' };
    const stdout = line.startsWith('loginctl show-user') ? 'yes\n'
      : line.endsWith('--property=MainPID') ? `MainPID=${control.active ? control.pid : 0}\n` : '';
    return { code: 0, stdout };
  };
  context.after(() => closeManaged(control));
  const home = join(root, 'home');
  const options = { configPath: join(home, 'framerize-remote.json'), env: { FRM_PORT: String(port) }, readyTimeoutMs: 3000,
    service: { host: { platform: 'linux', homeDir: root, uid: 501 }, home,
      program: ['/usr/bin/node', '/opt/frm/bin/frm.js', 'serve'], environmentPath: '/usr/bin', run } };
  const origin = `http://127.0.0.1:${remote.address().port}`;
  return { root, origin, seen, commands, control, options, port };
}

const pairLink = (origin, token) => `${origin}/pair#${new URLSearchParams([['token', token]])}`;
const serviceChanges = commands => commands.filter(command => !/ (is-|show-|show )/.test(command));

async function bounded(promise, milliseconds = 3000) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Operation did not settle')), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

const optionalBytes = path => readFile(path).catch(error => {
  if (error.code === 'ENOENT') return undefined;
  throw error;
});

for (const existing of [false, true]) test(`port lookup cancellation restores ${existing ? 'existing' : 'new'} setup`, {
  timeout: 8000,
}, async context => {
  const { origin, options, control, commands, root } = await fixture(context, { requireToken: null });
  if (existing) await setUp(origin, quiet, options);
  const unit = join(root, '.config/systemd/user/frm.service');
  const prior = { profile: await optionalBytes(options.configPath), unit: await optionalBytes(unit) };
  const controller = new AbortController();
  const reached = Promise.withResolvers();
  const original = dns.lookup;
  const hostname = 'pending-port.synthetic.invalid';
  let release, pending, observed, finalResult;
  dns.lookup = (host, settings, callback) => {
    if (host !== hostname) return original(host, settings, callback);
    const done = typeof settings === 'function' ? settings : callback;
    release = () => done(null, settings?.all ? [{ address: '127.0.0.1', family: 4 }] : '127.0.0.1', 4);
    reached.resolve();
  };
  syncBuiltinESMExports();
  try {
    pending = setUp(origin, quiet, { ...options, signal: controller.signal,
      env: { ...options.env, FRM_HOST: hostname } })
      .then(value => ({ value }), error => ({ error }));
    await bounded(Promise.race([reached.promise, pending.then(() => {
      throw new Error('Setup finished before the synthetic lookup');
    })]));
    controller.abort();
    const outcome = await bounded(pending, 750).catch(() => undefined);
    observed = { outcome, profile: await optionalBytes(options.configPath),
      unit: await optionalBytes(unit), active: control.active, commandCount: commands.length };
  } finally {
    dns.lookup = original;
    syncBuiltinESMExports();
    release?.();
    if (pending) finalResult = await bounded(pending);
  }
  assert.equal(finalResult.error?.code, 'aborted');
  assert.deepEqual(await optionalBytes(options.configPath), prior.profile);
  assert.deepEqual(await optionalBytes(unit), prior.unit);
  assert.equal(control.active, existing);
  assert.equal(observed.outcome?.error?.code, 'aborted', 'setup must settle before DNS is released');
  assert.deepEqual(observed.profile, prior.profile);
  assert.deepEqual(observed.unit, prior.unit);
  assert.equal(observed.active, existing);
  assert.equal(commands.length, observed.commandCount, 'late DNS must not change service state');
});

test('cancelled setup does not write a profile or run service commands', async context => {
  const { origin, options, commands } = await fixture(context, { requireToken: null });
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(setUp(origin, quiet, { ...options, signal: controller.signal }), { code: 'aborted' });
  await assert.rejects(readFile(options.configPath), { code: 'ENOENT' });
  assert.deepEqual(commands, []);
});

test('cancelling a service command permits uncancelled rollback', async context => {
  const { origin, options, control, root } = await fixture(context, { requireToken: null });
  const controller = new AbortController();
  const original = options.service.run;
  options.service.run = async (command, args, signal) => {
    assert.equal(signal, controller.signal.aborted ? undefined : controller.signal);
    const result = await original(command, args);
    if (args.includes('restart')) {
      await new Promise(resolve => {
        signal.addEventListener('abort', resolve, { once: true });
        queueMicrotask(() => controller.abort());
      });
    }
    return result;
  };
  await assert.rejects(setUp(origin, quiet, { ...options, signal: controller.signal }), { code: 'aborted' });
  assert.equal(control.active, false);
  await assert.rejects(readFile(options.configPath), { code: 'ENOENT' });
  await assert.rejects(readFile(join(root, '.config/systemd/user/frm.service')), { code: 'ENOENT' });
});

test('a failed rollback retains the setup failure and reports incomplete restoration', async context => {
  const { origin, options, control } = await fixture(context, { requireToken: null });
  control.listens = false;
  const original = options.service.run;
  options.service.run = async (command, args) => {
    const result = await original(command, args);
    if (args.includes('--property=MainPID')) control.fail = 'systemctl --user daemon-reload';
    return result;
  };
  await assert.rejects(setUp(origin, quiet, { ...options, readyTimeoutMs: 50 }), error => {
    assert.equal(error.code, 'service_rollback_failed');
    assert.equal(error.setupCode, 'service_not_ready');
    assert.equal(error.cause.errors.length, 2);
    return true;
  });
  assert.equal(control.active, false);
});

test('failed profile restoration does not restart the service with changed settings', async context => {
  const { origin, options, control } = await fixture(context, { requireToken: null });
  await setUp(origin, quiet, options);
  options.env.FRM_LOCAL_TOKEN = 'synthetic-new-local-token';
  control.listens = false;
  const original = options.service.run;
  let replaced = false;
  options.service.run = async (command, args, signal) => {
    const result = await original(command, args, signal);
    if (!replaced && args.includes('--property=MainPID')) {
      replaced = true;
      await rm(options.configPath);
      await mkdir(options.configPath);
    }
    return result;
  };
  await assert.rejects(setUp(origin, quiet, { ...options, readyTimeoutMs: 50 }), error => {
    assert.equal(error.code, 'service_rollback_failed');
    assert.equal(error.setupCode, 'service_not_ready');
    assert.equal(error.cause.errors.length, 2);
    return true;
  });
  assert.equal(control.active, false);
});

test('cancelling the managed listener wait restores the new profile and service', async context => {
  const { origin, options, control, root } = await fixture(context, { requireToken: null });
  const controller = new AbortController();
  control.listens = false;
  const original = options.service.run;
  options.service.run = async (command, args) => {
    const result = await original(command, args);
    if (args.includes('--property=MainPID')) controller.abort();
    return result;
  };
  await assert.rejects(setUp(origin, quiet, { ...options, signal: controller.signal }), { code: 'aborted' });
  await assert.rejects(readFile(options.configPath), { code: 'ENOENT' });
  await assert.rejects(readFile(join(root, '.config/systemd/user/frm.service')), { code: 'ENOENT' });
  assert.equal(control.active, false);
});

test('setup rejects a service that is inactive after its start command succeeds', async context => {
  const { origin, options, control } = await fixture(context, { requireToken: null });
  const original = options.service.run;
  options.service.run = async (command, args) => {
    const result = await original(command, args);
    if (command === 'systemctl' && args.includes('restart')) control.active = false;
    return result;
  };
  await assert.rejects(setUp(origin, quiet, options), { code: 'service_failed' });
  await assert.rejects(readFile(options.configPath), { code: 'ENOENT' });
});

test('an unchanged running service repairs disabled lingering without a restart', async context => {
  const { origin, commands, options } = await fixture(context, { requireToken: null });
  await setUp(origin, quiet, options);
  const saved = await readFile(options.configPath);
  commands.length = 0;
  const original = options.service.run;
  let lingering = false;
  options.service.run = async (command, args) => {
    const result = await original(command, args);
    if (command === 'loginctl' && args[0] === 'enable-linger') lingering = true;
    return command === 'loginctl' && args[0] === 'show-user'
      ? { code: 0, stdout: lingering ? 'yes\n' : 'no\n' } : result;
  };
  assert.equal((await setUp(origin, quiet, options)).service, 'unchanged');
  assert.equal(lingering, true);
  assert.ok(commands.includes('loginctl enable-linger --no-ask-password 501'));
  assert.ok(!commands.some(command => / (restart|stop|enable) frm\.service$/.test(command)));
  assert.deepEqual(await readFile(options.configPath), saved);
});

test('a pairing link saves the token inline, checks readiness, then installs and starts the service', async context => {
  const { origin, seen, commands, options, port } = await fixture(context);
  const result = await setUp(pairLink(origin, 'synthetic-pair+/='), quiet, options);
  assert.deepEqual(result, { remote: origin, paired: true, ready: true, service: 'installed',
    localApi: `http://127.0.0.1:${port}/v1` });
  const profile = JSON.parse(await readFile(options.configPath, 'utf8'));
  assert.deepEqual(profile.serverToken, { kind: 'value', value: 'synthetic-pair+/=' });
  assert.equal(profile.remote, origin);
  assert.deepEqual(seen, ['Bearer synthetic-pair+/=']);
  assert.deepEqual(commands.filter(command => !/ is-| show frm/.test(command)), ['systemctl --user show-environment',
    'loginctl show-user 501 --property=Linger --value', 'systemctl --user daemon-reload',
    'systemctl --user enable frm.service', 'systemctl --user restart frm.service']);
  assert.match(formatUp(result), new RegExp(`Service {5}installed and started\\n {2}Local API {3}http://127\\.0\\.0\\.1:${port}/v1`));
});

test('rerunning with the same link leaves a running service alone; a new link restarts it', async context => {
  const { origin, commands, options } = await fixture(context, { requireToken: null });
  await setUp(pairLink(origin, 'first'), quiet, options);
  commands.length = 0;
  assert.equal((await setUp(pairLink(origin, 'first'), quiet, options)).service, 'unchanged');
  assert.deepEqual(serviceChanges(commands), []);
  assert.equal((await setUp(pairLink(origin, 'second'), quiet, options)).service, 'restarted');
  assert.deepEqual(serviceChanges(commands), ['systemctl --user stop frm.service', 'systemctl --user start frm.service']);
});

test('a failed readiness check or service step restores the previous profile', async context => {
  const { origin, commands, control, options } = await fixture(context, { requireToken: null });
  await setUp(origin, quiet, options);
  const saved = await readFile(options.configPath, 'utf8');
  control.failOnce = 'systemctl --user start frm.service';
  commands.length = 0;
  await assert.rejects(setUp(pairLink(origin, 'changed'), quiet, options), { code: 'service_failed' });
  assert.equal(await readFile(options.configPath, 'utf8'), saved);
  assert.deepEqual(serviceChanges(commands), ['systemctl --user stop frm.service',
    'systemctl --user start frm.service', 'systemctl --user stop frm.service', 'systemctl --user daemon-reload',
    'systemctl --user enable frm.service', 'systemctl --user start frm.service']);
  assert.equal(control.active, true);
  const unready = await fixture(context, { requireToken: null, ready: false });
  commands.length = 0;
  await assert.rejects(setUp(unready.origin, quiet, unready.options), { code: 'upstream_unavailable' });
  await assert.rejects(readFile(unready.options.configPath), { code: 'ENOENT' });
  assert.deepEqual(unready.commands, []);
});

test('a rejected pairing token reports remote_auth and saves nothing', async context => {
  const { origin, commands, options } = await fixture(context);
  await assert.rejects(setUp(pairLink(origin, 'wrong'), quiet, options), { code: 'remote_auth' });
  await assert.rejects(readFile(options.configPath), { code: 'ENOENT' });
  assert.deepEqual(commands, []);
});

test('a plain origin keeps a saved token, and a missing address fails without a terminal', async context => {
  const { origin, options } = await fixture(context);
  await setUp(pairLink(origin, 'synthetic-pair+/='), quiet, options);
  assert.equal((await setUp(origin, quiet, options)).paired, false);
  assert.equal(JSON.parse(await readFile(options.configPath, 'utf8')).serverToken.value, 'synthetic-pair+/=');
  await assert.rejects(setUp(undefined, quiet, { ...options, configPath: join(options.service.home, 'absent.json') }),
    { code: 'remote_required' });
});

test('FRM token values are saved by value, so the service starts without shell variables', async context => {
  const { origin, seen, options } = await fixture(context, { requireToken: 'synthetic-env-server' });
  const env = { FRM_SERVER_TOKEN: 'synthetic-env-server', FRM_LOCAL_TOKEN: 'synthetic-env-local' };
  assert.equal((await setUp(origin, quiet, { ...options, env: { ...options.env, ...env } })).service, 'installed');
  const profile = JSON.parse(await readFile(options.configPath, 'utf8'));
  assert.deepEqual(profile.serverToken, { kind: 'value', value: 'synthetic-env-server' });
  assert.deepEqual(profile.localToken, { kind: 'value', value: 'synthetic-env-local' });
  assert.deepEqual(seen, ['Bearer synthetic-env-server']);
});

test('a pairing-link token wins over FRM_SERVER_TOKEN, which wins over a saved token', async context => {
  const { origin, options } = await fixture(context, { requireToken: null });
  const saved = () => readFile(options.configPath, 'utf8').then(text => JSON.parse(text).serverToken);
  await setUp(pairLink(origin, 'synthetic-link'), quiet, { ...options, env: { ...options.env, FRM_SERVER_TOKEN: 'synthetic-env' } });
  assert.deepEqual(await saved(), { kind: 'value', value: 'synthetic-link' });
  await setUp(origin, quiet, { ...options, env: { ...options.env, FRM_SERVER_TOKEN: 'synthetic-env' } });
  assert.deepEqual(await saved(), { kind: 'value', value: 'synthetic-env' });
  await setUp(origin, quiet, options);
  assert.deepEqual(await saved(), { kind: 'value', value: 'synthetic-env' });
  await setUp(`${origin}/pair`, quiet, options);
  assert.equal(await saved(), undefined);
});

test('empty or malformed FRM token values fail before saving or service changes', async context => {
  const { origin, commands, options } = await fixture(context, { requireToken: null });
  for (const [env, code] of [[{ FRM_SERVER_TOKEN: '' }, 'server_credential'],
    [{ FRM_LOCAL_TOKEN: '' }, 'local_credential'], [{ FRM_SERVER_TOKEN: 'bad token' }, 'server_credential']]) {
    await assert.rejects(setUp(origin, quiet, { ...options, env: { ...options.env, ...env } }), { code });
  }
  await assert.rejects(readFile(options.configPath), { code: 'ENOENT' });
  assert.deepEqual(commands, []);
});

test('a failed readiness check with an env token restores the previous profile', async context => {
  const { origin, options } = await fixture(context, { requireToken: 'synthetic-saved' });
  await setUp(origin, quiet, { ...options, env: { ...options.env, FRM_SERVER_TOKEN: 'synthetic-saved' } });
  const before = await readFile(options.configPath, 'utf8');
  await assert.rejects(setUp(origin, quiet, { ...options, env: { ...options.env, FRM_SERVER_TOKEN: 'synthetic-wrong' } }),
    { code: 'remote_auth' });
  assert.equal(await readFile(options.configPath, 'utf8'), before);
});

test('the local API address brackets IPv6 and maps wildcard hosts to loopback', async context => {
  const { origin, options } = await fixture(context, { requireToken: null });
  const { port } = options.env.FRM_PORT ? { port: options.env.FRM_PORT } : { port: 8788 };
  for (const [host, expected] of [['::1', `http://[::1]:${port}/v1`], ['::', `http://[::1]:${port}/v1`],
    ['0.0.0.0', `http://127.0.0.1:${port}/v1`], ['100.64.0.10', `http://100.64.0.10:${port}/v1`]]) {
    // Without a service manager, setup returns before binding, so these hosts need not exist here.
    const service = { ...options.service, host: { ...options.service.host, platform: 'win32' } };
    const result = await setUp(origin, quiet, { ...options, service, env: { ...options.env, FRM_HOST: host } });
    assert.equal(result.localApi, expected, host);
  }
});

test('a failed first setup removes its new service unit and profile', async context => {
  const { root, origin, commands, control, options } = await fixture(context, { requireToken: null });
  control.listens = false;
  await assert.rejects(setUp(origin, quiet, options), { code: 'service_not_ready' });
  await assert.rejects(readFile(options.configPath), { code: 'ENOENT' });
  await assert.rejects(readFile(join(root, '.config', 'systemd', 'user', 'frm.service')), { code: 'ENOENT' });
  assert.ok(commands.includes('systemctl --user disable --now frm.service'));
  assert.equal(control.active, false);
});

test('a failed change to a deliberately stopped service restores its bytes and leaves it stopped', async context => {
  const { root, origin, commands, control, options } = await fixture(context, { requireToken: null });
  await setUp(origin, quiet, options);
  const unitPath = join(root, '.config', 'systemd', 'user', 'frm.service');
  const [profile, unit] = await Promise.all([readFile(options.configPath, 'utf8'), readFile(unitPath, 'utf8')]);
  control.active = false;
  await closeManaged(control);
  control.listens = false;
  commands.length = 0;
  await assert.rejects(setUp(pairLink(origin, 'changed'), quiet, options), { code: 'service_not_ready' });
  assert.equal(await readFile(options.configPath, 'utf8'), profile);
  assert.equal(await readFile(unitPath, 'utf8'), unit);
  assert.equal(control.active, false);
  assert.equal(serviceChanges(commands).filter(command => / (re)?start frm\.service$/.test(command)).length, 1);
});

test('an occupied local port fails before activation and leaves the other listener alone', async context => {
  const { origin, commands, options, port } = await fixture(context, { requireToken: null });
  const other = createServer((request, response) => response.end('{"status":"ok"}'));
  await new Promise(resolve => other.listen(port, '127.0.0.1', resolve));
  context.after(() => new Promise(resolve => { other.closeAllConnections(); other.close(resolve); }));
  await assert.rejects(setUp(origin, quiet, options), { code: 'service_port_busy' });
  assert.equal(commands.some(command => / (re)?start frm\.service$/.test(command)), false);
  await assert.rejects(readFile(options.configPath), { code: 'ENOENT' });
  assert.equal(other.listening, true);
});
