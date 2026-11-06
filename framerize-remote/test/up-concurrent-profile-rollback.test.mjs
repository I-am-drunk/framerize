import assert from 'node:assert/strict';
import test from 'node:test';
import { createServer, get } from 'node:http';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build, stop } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { setUp } from './cmdUp.ts'; export { selectServiceManager } from './serviceManager.ts';",
  resolveDir: fileURLToPath(new URL('../src/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) } });
stop();
const bundle = compiled.outputFiles[0].contents;
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const { setUp, selectServiceManager } = await import(
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
  response.writeHead(200, { 'x-framerize-remote-pid': String(process.pid) });
  response.end('{"status":"ok"}');
});
server.listen(Number(process.argv[2]), '127.0.0.1', () => {
  process.send({ pid: process.pid, port: server.address().port });
});
process.on('SIGTERM', () => { server.closeAllConnections(); server.close(() => process.exit(0)); });
setTimeout(() => process.exit(9), 8000).unref();
`;
function readyResponse(response, ready) {
  response.writeHead(ready ? 200 : 503, { 'content-type': 'application/json' });
  response.end(JSON.stringify({ apiVersion: 1, status: ready ? 'ready' : 'unavailable',
    capabilities: { responses: true, chatCompletions: true, streaming: true },
    ...(ready ? {} : { reason: 'upstream_unavailable' }) }));
}

for (const [existing, cancelled] of [[false, false], [true, false], [true, true]])
test(`concurrent up retries an ${existing ? 'existing' : 'absent'} profile after ${cancelled ? 'cancellation' : 'failure'}`,
  { timeout: 10_000 }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'frm-concurrent-profile-'));
    const commands = [], output = [], requests = [];
    const reached = Promise.withResolvers();
    const controller = new AbortController();
    const retryController = new AbortController();
    let held, child, closed, pending, later, local, cleaned = false;
    const first = createServer((request, response) => {
      requests.push(['first', request.url]);
      held = response;
      reached.resolve();
    });
    const second = createServer((request, response) => {
      requests.push(['second', request.url]);
      readyResponse(response, true);
    });
    const reservation = createServer();
    const alive = () => Boolean(child && child.exitCode === null && child.signalCode === null);
    const origins = [];
    const cleanup = async () => {
      if (cleaned) return;
      controller.abort();
      retryController.abort();
      if (held && !held.writableEnded && !held.destroyed) readyResponse(held, false);
      await bounded(Promise.allSettled([pending, later].filter(Boolean)));
      if (alive()) child.kill('SIGKILL');
      if (closed) await bounded(closed);
      for (const server of [first, second, reservation]) {
        server.closeAllConnections();
        await bounded(new Promise(resolve => server.close(resolve)));
      }
      for (const origin of [...origins, ...(local ? [local] : [])]) {
        await assert.rejects(status(`${origin}/health`), { code: 'ECONNREFUSED' });
      }
      await rm(root, { recursive: true, force: true });
      cleaned = true;
    };
    context.after(cleanup);
    for (const server of [first, second, reservation]) {
      server.listen(0, '127.0.0.1');
      await bounded(once(server, 'listening'));
      if (server !== reservation) origins.push(`http://127.0.0.1:${server.address().port}`);
    }
    const port = reservation.address().port;
    await bounded(new Promise(resolve => reservation.close(resolve)));
    local = `http://127.0.0.1:${port}`;
    const home = join(root, 'profile');
    const configPath = join(home, 'framerize-remote.json');
    await mkdir(home, { mode: 0o700 });
    const prior = ' { "remote": "http://127.0.0.1:9", "port": 8788 }\n';
    if (existing) await writeFile(configPath, prior, { mode: 0o600 });
    const host = { platform: 'linux', homeDir: root, xdgConfigHome: join(root, 'config'), uid: 7342 };
    const manager = selectServiceManager(host);
    const executable = join(root, 'managed.mjs');
    await writeFile(executable, childSource, { mode: 0o600 });
    const service = { home, host, environmentPath: '/synthetic/bin',
      program: [process.execPath, executable] };
    service.run = async (command, args) => {
      commands.push([command, ...args]);
      if (command === 'loginctl') return { code: 0, stdout: 'yes\n' };
      assert.equal(command, 'systemctl');
      const action = args[1];
      if (action === 'is-active') return { code: alive() ? 0 : 3, stdout: '' };
      if (action === 'show') return { code: 0, stdout: `MainPID=${alive() ? child.pid : 0}\n` };
      if (['show-environment', 'is-enabled', 'daemon-reload', 'enable'].includes(action)) {
        return { code: 0, stdout: '' };
      }
      if (action === 'restart') {
        assert.equal(child, undefined, 'Only the newer setup may activate a service');
        assert.equal(JSON.parse(await readFile(configPath, 'utf8')).remote, origins[1]);
        child = spawn(process.execPath, [executable, String(port)], {
          env: { HOME: root, USERPROFILE: root, TMPDIR: root },
          stdio: ['ignore', 'ignore', 'ignore', 'ipc'],
        });
        closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })));
        const [ready] = await bounded(once(child, 'message'));
        assert.deepEqual(ready, { pid: child.pid, port });
        return { code: 0, stdout: '' };
      }
      if (action === 'stop' || action === 'disable') {
        if (alive()) child.kill('SIGKILL');
        if (closed) await bounded(closed);
        return { code: 0, stdout: '' };
      }
      assert.fail(`Unexpected synthetic service action: ${action}`);
    };
    const options = { configPath, service, readyTimeoutMs: 1000, signal: controller.signal,
      env: { HOME: root, USERPROFILE: root, FRM_HOME: home, FRM_PORT: String(port) } };
    const io = { stdout: text => output.push(text), stderr: text => output.push(text) };
    pending = setUp(origins[0], io, options).then(value => ({ value }), error => ({ error }));
    await bounded(Promise.race([reached.promise, pending.then(() => {
      throw new Error('Older setup settled before its readiness request');
    })]));
    const waitingProfile = await readFile(configPath);
    const waitingCommands = commands.length;
    await assert.rejects(bounded(setUp(origins[1], io, options)), { code: 'profile_busy' });
    assert.deepEqual(await readFile(configPath), waitingProfile);
    assert.equal(commands.length, waitingCommands);
    assert.deepEqual(requests, [['first', '/ready']]);
    if (cancelled) controller.abort();
    else readyResponse(held, false);
    const older = await bounded(pending);
    assert.deepEqual(await optionalBytes(configPath), existing ? Buffer.from(prior) : undefined);
    assert.equal(await optionalBytes(join(home, '.framerize-remote.json.lock')), undefined);
    later = setUp(origins[1], io, { ...options, signal: retryController.signal });
    const newer = await bounded(later, 4000);
    assert.equal(newer.ready, true);
    assert.equal(newer.service, 'installed');
    assert.equal(newer.remote, origins[1]);
    const completed = await readFile(configPath);
    assert.equal(JSON.parse(completed).remote, origins[1]);
    const completedUnit = await readFile(manager.unitPath);
    const commandCount = commands.length;
    assert.equal(await status(`${local}/health`), 200);
    const finalProfile = await optionalBytes(configPath);
    const unitUnchanged = (await readFile(manager.unitPath)).equals(completedUnit);
    const stillRunning = alive() && await status(`${local}/health`) === 200;
    const preserved = finalProfile?.equals(completed) ?? false;
    await cleanup();
    context.diagnostic(JSON.stringify({ existing, olderCode: older.error?.code,
      newerReady: newer.ready, newerService: newer.service, profileRemoved: finalProfile === undefined,
      originalRestored: finalProfile?.equals(Buffer.from(prior)) ?? false,
      newerProfilePreserved: preserved, unitUnchanged, stillRunning,
      completedProfileSha256: hash(completed), finalProfileSha256: finalProfile && hash(finalProfile),
      requests, cleanupComplete: cleaned, bundleSha256: hash(bundle), inputs: Object.keys(compiled.metafile.inputs) }));
    assert.equal(older.error?.code, cancelled ? 'aborted' : 'upstream_unavailable');
    assert.equal(older.value, undefined);
    assert.equal(unitUnchanged, true);
    assert.equal(stillRunning, true);
    assert.equal(commands.length, commandCount);
    assert.deepEqual(requests, [['first', '/ready'], ['second', '/ready']]);
    assert.deepEqual(output, []);
    assert.equal(preserved, true, 'A failed older setup must not overwrite a completed newer profile');
  });
