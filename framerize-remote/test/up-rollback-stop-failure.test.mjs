import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer, get } from 'node:http';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build, stop } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { setUp } from './cmdUp.ts'; export { restoreService } from './serviceControl.ts'; export { selectServiceManager } from './serviceManager.ts';",
  resolveDir: fileURLToPath(new URL('../src/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) } });
stop();
const bundle = compiled.outputFiles[0].contents;
const bundleSha256 = createHash('sha256').update(bundle).digest('hex');
const { setUp, restoreService, selectServiceManager } = await import(
  `data:text/javascript;base64,${Buffer.from(bundle).toString('base64')}`);

async function bounded(pending, milliseconds = 2000) {
  let timer;
  try { return await Promise.race([pending, new Promise((resolveUnused, reject) => {
    timer = setTimeout(() => reject(new Error('Owned fixture exceeded its wait bound')), milliseconds);
  })]); } finally { clearTimeout(timer); }
}

function status(url) {
  return new Promise((resolve, reject) => {
    const request = get(url, { agent: false }, response => {
      response.resume();
      response.once('end', () => resolve(response.statusCode));
      response.once('error', reject);
    });
    request.setTimeout(1000, () => request.destroy(new Error('Owned request timed out')));
    request.once('error', reject);
  });
}

const optionalBytes = path => readFile(path).catch(error => {
  if (error.code === 'ENOENT') return undefined;
  throw error;
});
const childSource = `import { createServer } from 'node:http';
const server = createServer((request, response) => {
  response.writeHead(request.url === '/health' ? 503 : 200,
    { 'x-framerize-remote-pid': String(process.pid) });
  response.end('synthetic managed listener');
});
server.listen(Number(process.argv[2]), '127.0.0.1', () => {
  process.send({ pid: process.pid, port: server.address().port });
});
process.on('SIGTERM', () => { server.closeAllConnections(); server.close(() => process.exit(0)); });
setTimeout(() => process.exit(9), 8000).unref();
`;

for (const refuseStop of [false, true]) test(`up rollback ${refuseStop ? 'reports a refused' : 'completes a successful'} managed stop`,
  { timeout: 10_000 }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'frm-up-rollback-stop-'));
    const commands = [], requests = [], output = [];
    const upstream = createServer((request, response) => {
      requests.push(request.url);
      response.setHeader('content-type', 'application/json');
      response.end(JSON.stringify({ apiVersion: 1, status: 'ready',
        capabilities: { responses: true, chatCompletions: true, streaming: true } }));
    });
    const reservation = createServer();
    const controller = new AbortController();
    let child, closed, origin, local, pending, cleaned = false, unitAtStop;
    const alive = () => Boolean(child && child.exitCode === null && child.signalCode === null);
    const cleanup = async () => {
      if (cleaned) return;
      controller.abort();
      if (alive()) child.kill('SIGKILL');
      if (closed) await bounded(closed);
      if (pending) await bounded(pending.catch(() => {}));
      upstream.closeAllConnections();
      reservation.closeAllConnections();
      await Promise.all([upstream, reservation].map(server =>
        bounded(new Promise(resolve => server.close(resolve)))));
      if (local) await assert.rejects(status(`${local}/owned`), { code: 'ECONNREFUSED' });
      if (origin) await assert.rejects(status(`${origin}/ready`), { code: 'ECONNREFUSED' });
      await rm(root, { recursive: true, force: true });
      cleaned = true;
    };
    context.after(cleanup);
    upstream.listen(0, '127.0.0.1');
    await bounded(once(upstream, 'listening'));
    origin = `http://127.0.0.1:${upstream.address().port}`;
    reservation.listen(0, '127.0.0.1');
    await bounded(once(reservation, 'listening'));
    const port = reservation.address().port;
    await bounded(new Promise(resolve => reservation.close(resolve)));
    local = `http://127.0.0.1:${port}`;
    const home = join(root, 'profile');
    const configPath = join(home, 'framerize-remote.json');
    const host = { platform: 'linux', homeDir: root, xdgConfigHome: join(root, 'config'), uid: 7342 };
    const manager = selectServiceManager(host);
    const executable = join(root, 'managed.mjs');
    await writeFile(executable, childSource, { mode: 0o600 });
    const startChild = async () => {
      assert.equal(child, undefined, 'Only one activation is expected');
      child = spawn(process.execPath, [executable, String(port)], {
        env: { HOME: root, USERPROFILE: root, TMPDIR: root },
        stdio: ['ignore', 'pipe', 'pipe', 'ipc'],
      });
      closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })));
      child.stdout.resume();
      child.stderr.resume();
      const [ready] = await bounded(once(child, 'message'));
      assert.deepEqual(ready, { pid: child.pid, port });
      assert.equal(alive(), true);
      assert.equal(await status(`${local}/owned`), 200);
    };
    const service = { home, host, environmentPath: '/synthetic/bin',
      program: [process.execPath, executable], async run(command, args) {
        commands.push([command, ...args]);
        if (command === 'loginctl') {
          assert.equal(args[0], 'show-user');
          return { code: 0, stdout: 'yes\n' };
        }
        assert.equal(command, 'systemctl');
        assert.equal(args[0], '--user');
        const action = args[1];
        if (action === 'is-active') return { code: alive() ? 0 : 3, stdout: '' };
        if (action === 'show') return { code: 0, stdout: `MainPID=${alive() ? child.pid : 0}\n` };
        if (['show-environment', 'is-enabled', 'daemon-reload', 'enable'].includes(action)) {
          return { code: 0, stdout: '' };
        }
        if (action === 'restart') { await startChild(); return { code: 0, stdout: '' }; }
        if (action === 'disable' || action === 'stop') {
          unitAtStop = await readFile(manager.unitPath);
          assert.equal(alive(), true);
          if (refuseStop) return { code: 1, stdout: 'synthetic stop refused' };
          child.kill('SIGTERM');
          const stopped = await bounded(closed);
          if (process.platform === 'win32') assert.ok(stopped.code === 0 || stopped.signal === 'SIGTERM');
          else assert.deepEqual(stopped, { code: 0, signal: null });
          return { code: 0, stdout: '' };
        }
        assert.fail(`Unexpected synthetic service action: ${action}`);
      } };
    const io = { stdout: text => output.push(text), stderr: text => output.push(text) };
    let result, failure;
    try {
      pending = setUp(origin, io, { configPath, service, readyTimeoutMs: 300, signal: controller.signal,
        env: { HOME: root, USERPROFILE: root, FRM_HOME: home, FRM_PORT: String(port) } });
      result = await bounded(pending, 4000);
    } catch (error) { failure = error; }
    const unitAfter = await optionalBytes(manager.unitPath);
    const profileAfter = await optionalBytes(configPath);
    const childAlive = alive();
    const listening = await status(`${local}/owned`).then(code => code === 200, error => {
      assert.equal(error.code, 'ECONNREFUSED');
      return false;
    });
    await cleanup();
    context.diagnostic(JSON.stringify({ refuseStop, code: failure?.code, setupCode: failure?.setupCode,
      childAlive, listening, unitRemoved: unitAfter === undefined, profileRemoved: profileAfter === undefined,
      commands, cleanupComplete: cleaned, bundleSha256, inputs: Object.keys(compiled.metafile.inputs) }));
    assert.equal(result, undefined);
    assert.equal(childAlive, refuseStop);
    assert.equal(listening, refuseStop);
    assert.equal(profileAfter, undefined);
    assert.deepEqual(output, []);
    assert.deepEqual(requests, ['/ready']);
    assert.ok(unitAtStop?.length > 0);
    assert.equal(failure?.code, refuseStop ? 'service_rollback_failed' : 'service_not_ready');
    if (refuseStop) {
      assert.equal(failure.setupCode, 'service_not_ready');
      assert.ok(failure.cause?.errors.some(error => error.code === 'service_failed'));
      assert.deepEqual(unitAfter, unitAtStop, 'A live service must retain its management unit');
    } else assert.equal(unitAfter, undefined);
  });

for (const [platform, label, state, missing] of [
  ['linux', 'missing', { code: 0, stdout: 'LoadState=not-found\nActiveState=inactive\nMainPID=0\n' }, true],
  ['linux', 'loaded', { code: 0, stdout: 'LoadState=loaded\nActiveState=inactive\nMainPID=0\n' }, false],
  ['linux', 'still active', { code: 0, stdout: 'LoadState=not-found\nActiveState=active\nMainPID=1234\n' }, false],
  ['linux', 'unavailable', { code: 1, stdout: '' }, false],
  ['darwin', 'missing', { code: 113, stdout: '' }, true],
  ['darwin', 'loaded', { code: 0, stdout: 'state = waiting\n' }, false],
  ['darwin', 'unavailable', { code: 1, stdout: '' }, false],
  ['darwin', 'missing domain', { code: 3, stdout: '' }, false],
]) test(`Remote rollback keeps failed-stop files unless ${platform} confirms absence: ${label}`, async t => {
  const root = await mkdtemp(join(tmpdir(), 'remote-rollback-state-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const host = { platform, homeDir: root, uid: 7342 };
  const manager = selectServiceManager(host);
  const current = 'synthetic current unit\n';
  await mkdir(dirname(manager.unitPath), { recursive: true });
  await writeFile(manager.unitPath, current);
  const commands = [];
  const service = { host, home: join(root, 'profile'), program: [], environmentPath: '',
    async run(command, args) {
      const action = command === 'systemctl' ? args[1] : args[0];
      commands.push(action);
      if (action === 'disable' || action === 'bootout') return { code: 1, stdout: '' };
      if (action === 'show' || action === 'print') return state;
      assert.equal(action, 'daemon-reload');
      return { code: 0, stdout: '' };
    } };
  const failure = await restoreService(service, { unit: undefined, enabled: false, running: false })
    .then(() => undefined, error => error);
  if (missing) {
    assert.equal(failure, undefined);
    assert.equal(await optionalBytes(manager.unitPath), undefined);
  } else {
    assert.equal(failure?.code, 'service_failed');
    assert.equal(await readFile(manager.unitPath, 'utf8'), current);
  }
  assert.deepEqual(commands, platform === 'linux'
    ? ['disable', 'show', ...(missing ? ['daemon-reload'] : [])] : ['bootout', 'print']);
});
