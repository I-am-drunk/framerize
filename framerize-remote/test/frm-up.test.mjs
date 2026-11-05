// The restart and rollback cases adapt T3 Code apps/server/src/cli/up.test.ts at 5a906583 (MIT, T3 Tools Inc.).
import test from 'node:test';
import assert from 'node:assert/strict';
import { createServer } from 'node:http';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ entryPoints: [fileURLToPath(new URL('../src/cmdUp.ts', import.meta.url))],
  alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) },
  bundle: true, platform: 'node', format: 'esm', write: false });
const { setUp, formatUp } = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const quiet = { stdout() {}, stderr() {} };

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
  const control = { active: true, fail: undefined };
  const run = async (command, args) => {
    const line = `${command} ${args.join(' ')}`;
    commands.push(line);
    if (/ (re)?start frm\.service$/.test(line)) control.active = true;
    const failed = line === control.fail || (line.includes(' is-active ') && !control.active);
    return { code: failed ? 1 : 0, stdout: line.startsWith('loginctl show-user') ? 'yes\n' : '' };
  };
  const home = join(root, 'home');
  const options = { configPath: join(home, 'framerize-remote.json'), env: {},
    service: { host: { platform: 'linux', homeDir: root, uid: 501 }, home,
      program: ['/usr/bin/node', '/opt/frm/bin/frm.js', 'serve'], environmentPath: '/usr/bin', run } };
  const origin = `http://127.0.0.1:${remote.address().port}`;
  return { root, origin, seen, commands, control, options };
}

const pairLink = (origin, token) => `${origin}/pair#${new URLSearchParams([['token', token]])}`;
const serviceChanges = commands => commands.filter(command => !/ (is-|show-)/.test(command));

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
  const { origin, seen, commands, options } = await fixture(context);
  const result = await setUp(pairLink(origin, 'synthetic-pair+/='), quiet, options);
  assert.deepEqual(result, { remote: origin, paired: true, ready: true, service: 'installed',
    localApi: 'http://127.0.0.1:8788/v1' });
  const profile = JSON.parse(await readFile(options.configPath, 'utf8'));
  assert.deepEqual(profile.serverToken, { kind: 'value', value: 'synthetic-pair+/=' });
  assert.equal(profile.remote, origin);
  assert.deepEqual(seen, ['Bearer synthetic-pair+/=']);
  assert.deepEqual(commands.filter(command => !command.includes(' is-')), ['systemctl --user show-environment',
    'loginctl show-user 501 --property=Linger --value', 'systemctl --user daemon-reload',
    'systemctl --user enable frm.service', 'systemctl --user restart frm.service']);
  assert.match(formatUp(result), /Service {5}installed and started\n {2}Local API {3}http:\/\/127\.0\.0\.1:8788\/v1/);
});

test('rerunning with the same link leaves a running service alone; a new link restarts it', async context => {
  const { origin, commands, options } = await fixture(context, { requireToken: null });
  await setUp(pairLink(origin, 'first'), quiet, options);
  commands.length = 0;
  assert.equal((await setUp(pairLink(origin, 'first'), quiet, options)).service, 'unchanged');
  assert.deepEqual(serviceChanges(commands), []);
  assert.equal((await setUp(pairLink(origin, 'second'), quiet, options)).service, 'restarted');
  assert.deepEqual(serviceChanges(commands), ['systemctl --user restart frm.service']);
});

test('a failed readiness check or service step restores the previous profile', async context => {
  const { origin, commands, control, options } = await fixture(context, { requireToken: null });
  await setUp(origin, quiet, options);
  const saved = await readFile(options.configPath, 'utf8');
  control.fail = 'systemctl --user restart frm.service';
  commands.length = 0;
  await assert.rejects(setUp(pairLink(origin, 'changed'), quiet, options), { code: 'service_failed' });
  assert.equal(await readFile(options.configPath, 'utf8'), saved);
  assert.deepEqual(serviceChanges(commands),
    ['systemctl --user restart frm.service', 'systemctl --user restart frm.service']);
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
  assert.equal((await setUp(origin, quiet, { ...options, env })).service, 'installed');
  const profile = JSON.parse(await readFile(options.configPath, 'utf8'));
  assert.deepEqual(profile.serverToken, { kind: 'value', value: 'synthetic-env-server' });
  assert.deepEqual(profile.localToken, { kind: 'value', value: 'synthetic-env-local' });
  assert.deepEqual(seen, ['Bearer synthetic-env-server']);
});

test('a pairing-link token wins over FRM_SERVER_TOKEN, which wins over a saved token', async context => {
  const { origin, options } = await fixture(context, { requireToken: null });
  const saved = () => readFile(options.configPath, 'utf8').then(text => JSON.parse(text).serverToken);
  await setUp(pairLink(origin, 'synthetic-link'), quiet, { ...options, env: { FRM_SERVER_TOKEN: 'synthetic-env' } });
  assert.deepEqual(await saved(), { kind: 'value', value: 'synthetic-link' });
  await setUp(origin, quiet, { ...options, env: { FRM_SERVER_TOKEN: 'synthetic-env' } });
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
    await assert.rejects(setUp(origin, quiet, { ...options, env }), { code });
  }
  await assert.rejects(readFile(options.configPath), { code: 'ENOENT' });
  assert.deepEqual(commands, []);
});

test('a failed readiness check with an env token restores the previous profile', async context => {
  const { origin, options } = await fixture(context, { requireToken: 'synthetic-saved' });
  await setUp(origin, quiet, { ...options, env: { FRM_SERVER_TOKEN: 'synthetic-saved' } });
  const before = await readFile(options.configPath, 'utf8');
  await assert.rejects(setUp(origin, quiet, { ...options, env: { FRM_SERVER_TOKEN: 'synthetic-wrong' } }),
    { code: 'remote_auth' });
  assert.equal(await readFile(options.configPath, 'utf8'), before);
});

test('the local API address brackets IPv6 and maps wildcard hosts to loopback', async context => {
  const { origin, options } = await fixture(context, { requireToken: null });
  for (const [host, expected] of [['::1', 'http://[::1]:8788/v1'], ['::', 'http://[::1]:8788/v1'],
    ['0.0.0.0', 'http://127.0.0.1:8788/v1'], ['100.64.0.10', 'http://100.64.0.10:8788/v1']]) {
    const result = await setUp(origin, quiet, { ...options, env: { FRM_HOST: host } });
    assert.equal(result.localApi, expected, host);
  }
});
