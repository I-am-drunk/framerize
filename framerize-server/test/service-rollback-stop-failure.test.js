import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer, get } from 'node:http';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build, stop } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { reconcileService, runStep, ServerSetupError } from './service-control.ts'; export { selectServiceManager } from './service-units.ts';",
  resolveDir: fileURLToPath(new URL('../src/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
stop();
const bundle = compiled.outputFiles[0].contents;
const bundleSha256 = createHash('sha256').update(bundle).digest('hex');
const { reconcileService, runStep, ServerSetupError, selectServiceManager } = await import(
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
  response.writeHead(request.url === '/health' ? 503 : 200);
  response.end('synthetic managed listener');
});
server.listen(Number(process.argv[2]), '127.0.0.1', () => {
  process.send({ pid: process.pid, port: server.address().port });
});
process.on('SIGTERM', () => { server.closeAllConnections(); server.close(() => process.exit(0)); });
setTimeout(() => process.exit(9), 8000).unref();
`;

for (const refuseStop of [false, true]) test(`Server rollback ${refuseStop ? 'reports a refused' : 'completes a successful'} managed stop`,
  { timeout: 10_000 }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'server-rollback-stop-'));
    const commands = [];
    const reservation = createServer();
    const controller = new AbortController();
    let child, closed, local, pending, unitAtStop, settingsAtStop, cleaned = false, enabled = false;
    const alive = () => Boolean(child && child.exitCode === null && child.signalCode === null);
    const cleanup = async () => {
      if (cleaned) return;
      controller.abort();
      if (alive()) child.kill('SIGKILL');
      if (closed) await bounded(closed);
      if (pending) await bounded(pending.catch(() => {}));
      reservation.closeAllConnections();
      await bounded(new Promise(resolve => reservation.close(resolve)));
      if (local) await assert.rejects(status(`${local}/owned`), { code: 'ECONNREFUSED' });
      await rm(root, { recursive: true, force: true });
      cleaned = true;
    };
    context.after(cleanup);
    reservation.listen(0, '127.0.0.1');
    await bounded(once(reservation, 'listening'));
    const port = reservation.address().port;
    await bounded(new Promise(resolve => reservation.close(resolve)));
    local = `http://127.0.0.1:${port}`;
    const home = join(root, 'profile');
    const settingsPath = join(home, 'server.json');
    const executable = join(root, 'managed.mjs');
    await writeFile(executable, childSource, { mode: 0o600 });
    const plan = { home, homeDir: root, xdgConfigHome: join(root, 'config'),
      platform: 'linux', uid: 7342, program: [process.execPath, executable],
      environmentPath: '/synthetic/bin' };
    const manager = selectServiceManager(plan);
    const settings = JSON.stringify({ backend: 'http', host: '127.0.0.1', port });
    const primary = new ServerSetupError('Synthetic managed listener is not ready');
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
    const run = async (command, args) => {
      commands.push([command, ...args]);
      assert.equal(command, 'systemctl');
      assert.equal(args[0], '--user');
      const action = args[1];
      if (action === 'is-enabled') return { code: enabled ? 0 : 1, stdout: '' };
      if (action === 'enable') { enabled = true; return { code: 0, stdout: '' }; }
      if (action === 'daemon-reload') return { code: 0, stdout: '' };
      if (action === 'show') return { code: 0, stdout: alive()
        ? `ActiveState=active\nSubState=running\nMainPID=${child.pid}\n`
        : 'ActiveState=inactive\nSubState=dead\nMainPID=0\n' };
      if (action === 'restart') { await startChild(); return { code: 0, stdout: '' }; }
      if (action === 'disable') {
        unitAtStop = await readFile(manager.path);
        settingsAtStop = await readFile(settingsPath);
        assert.equal(alive(), true);
        if (refuseStop) return { code: 1, stdout: 'synthetic stop refused' };
        assert.equal(child.kill('SIGTERM'), true);
        const stopped = await bounded(closed);
        if (process.platform === 'win32') assert.ok(stopped.code === 0 || stopped.signal === 'SIGTERM');
        else assert.deepEqual(stopped, { code: 0, signal: null });
        enabled = false;
        return { code: 0, stdout: '' };
      }
      assert.fail(`Unexpected synthetic service action: ${action}`);
    };
    let result, failure;
    try {
      pending = reconcileService(plan, settings, async () => {
        assert.equal(await status(`${local}/health`), 503);
        throw primary;
      }, run, async () => {}, controller.signal);
      result = await bounded(pending, 4000);
    } catch (error) { failure = error; }
    const unitAfter = await optionalBytes(manager.path);
    const settingsAfter = await optionalBytes(settingsPath);
    const childAlive = alive();
    const listening = await status(`${local}/owned`).then(code => code === 200, error => {
      assert.equal(error.code, 'ECONNREFUSED');
      return false;
    });
    await cleanup();
    context.diagnostic(JSON.stringify({ refuseStop, error: failure?.message,
      sameError: failure === primary, childAlive, listening, unitRemoved: unitAfter === undefined,
      settingsRemoved: settingsAfter === undefined, commands, cleanupComplete: cleaned,
      bundleSha256, inputs: Object.keys(compiled.metafile.inputs) }));
    assert.equal(result, undefined);
    assert.equal(childAlive, refuseStop);
    assert.equal(listening, refuseStop);
    assert.deepEqual(unitAtStop, Buffer.from(manager.contents));
    assert.deepEqual(settingsAtStop, Buffer.from(settings));
    if (refuseStop) {
      assert.ok(failure?.cause instanceof AggregateError, 'Incomplete rollback must preserve its failure');
      assert.ok(failure.cause.errors.includes(primary));
      assert.match(failure.message, /Rollback also failed/);
      assert.deepEqual(unitAfter, unitAtStop, 'A live service must retain its management unit');
      assert.deepEqual(settingsAfter, settingsAtStop, 'Keep the settings referenced by the live unit');
    } else {
      assert.equal(failure, primary);
      assert.equal(unitAfter, undefined);
      assert.equal(settingsAfter, undefined);
    }
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
]) test(`Server rollback keeps failed-stop files unless ${platform} confirms absence: ${label}`, async t => {
  const root = await mkdtemp(join(tmpdir(), 'server-rollback-state-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  const plan = { platform, homeDir: root, uid: 7342, home: join(root, 'profile'),
    program: ['/synthetic/node', '/synthetic/server'], environmentPath: '/synthetic/bin' };
  const manager = selectServiceManager(plan);
  const settingsPath = join(plan.home, 'server.json');
  const settings = '{"synthetic":"new settings"}\n';
  const primary = new ServerSetupError('Synthetic readiness failure');
  const commands = [];
  const run = async (command, args) => {
    const action = command === 'systemctl' ? args[1] : args[0];
    commands.push(action);
    if (action === 'disable' || action === 'bootout') return { code: 1, stdout: '' };
    if (action === 'show' || action === 'print') return state;
    assert.ok(['is-enabled', 'daemon-reload', 'enable', 'restart', 'bootstrap'].includes(action));
    return { code: 0, stdout: '' };
  };
  const failure = await reconcileService(plan, settings, async () => { throw primary; }, run)
    .then(() => undefined, error => error);
  if (missing) {
    assert.equal(failure, primary);
    assert.equal(await optionalBytes(manager.path), undefined);
    assert.equal(await optionalBytes(settingsPath), undefined);
  } else {
    assert.ok(failure?.cause instanceof AggregateError);
    assert.ok(failure.cause.errors.includes(primary));
    assert.equal(await readFile(manager.path, 'utf8'), manager.contents);
    assert.equal(await readFile(settingsPath, 'utf8'), settings);
  }
  const stop = commands.indexOf(platform === 'linux' ? 'disable' : 'bootout');
  assert.deepEqual(commands.slice(stop), platform === 'linux'
    ? ['disable', 'show', ...(missing ? ['daemon-reload'] : [])] : ['bootout', 'print']);
});

for (const code of [0, 3, 113]) test(`Server runner preserves completed command exit ${code}`, async () => {
  const result = await runStep(process.execPath, ['-e', `process.exit(${code})`]);
  assert.equal(result.code, code);
});

test('Server runner reports forced output cancellation as failure', async () => {
  const result = await runStep(process.execPath,
    ['-e', 'process.stdout.write("x".repeat(300000), () => process.exit(113))']);
  assert.equal(result.code, 1);
  assert.ok(Buffer.byteLength(result.stdout) <= 256 * 1024);
});
