import test from 'node:test';
import assert from 'node:assert/strict';
import childProcess, { spawn } from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
import { EventEmitter, once } from 'node:events';
import { mkdtemp, mkdir, readFile, rm } from 'node:fs/promises';
import { createServer as createHttpServer, request } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const packageRoot = (key, name) => process.env[key]
  ?? fileURLToPath(new URL(`../${name}/`, import.meta.url));
const serverRoot = packageRoot('FRAMERIZE_SERVER_PACKAGE_ROOT', 'framerize-server');
const remoteRoot = packageRoot('FRAMERIZE_REMOTE_PACKAGE_ROOT', 'framerize-remote');
const fromPackage = (root, file) => import(pathToFileURL(join(root, file)).href);

async function bounded(promise, description) {
  let timer;
  try { return await Promise.race([promise, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(description)), 5000);
  })]); } finally { clearTimeout(timer); }
}

async function reservePort() {
  const server = createHttpServer();
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return server;
}

function exchange(port, path, token, body) {
  return new Promise((resolve, reject) => {
    const outgoing = request({ host: '127.0.0.1', port, path, agent: false,
      method: body ? 'POST' : 'GET', signal: AbortSignal.timeout(2500),
      headers: { ...(token ? { authorization: `Bearer ${token}` } : {}),
        ...(body ? { 'content-type': 'application/json' } : {}) } }, response => {
      const chunks = [];
      response.on('data', chunk => chunks.push(chunk));
      response.once('error', reject);
      response.once('end', () => resolve({ status: response.statusCode,
        body: Buffer.concat(chunks).toString() }));
    });
    outgoing.once('error', reject);
    outgoing.end(body && JSON.stringify(body));
  });
}

for (const token of [undefined, 'synthetic-pair+/=']) {
  test(`Server pairing drives a saved frm service with ${token ? 'a token' : 'open access'}`,
    { skip: process.platform !== 'linux', timeout: 30000 }, async context => {
      const root = await mkdtemp(join(tmpdir(), 'frm-pair-flow-'));
      const home = join(root, 'profile');
      const reservation = await reservePort();
      const { port } = reservation.address();
      const localToken = token ? 'synthetic-local-only' : undefined;
      const previousEnv = process.env;
      const originalExecFile = childProcess.execFile;
      let server, child, closed, ready, output = '', starts = 0, enabled = false;
      const env = { HOME: root, USERPROFILE: root, XDG_CONFIG_HOME: join(root, 'config'),
        FRM_HOME: home, FRM_PORT: String(port), NODE_DISABLE_COMPILE_CACHE: '1', PATH: '',
        ...(localToken ? { FRM_LOCAL_TOKEN: localToken } : {}) };
      const running = () => child && child.exitCode === null && child.signalCode === null;
      async function stop() {
        if (running()) child.kill('SIGTERM');
        if (closed) assert.deepEqual(await bounded(closed, 'frm did not stop'), { code: 0, signal: null }, output);
      }
      context.after(async () => {
        process.env = previousEnv;
        childProcess.execFile = originalExecFile;
        syncBuiltinESMExports();
        try { await stop(); }
        finally {
          if (reservation.listening) await new Promise(resolve => reservation.close(resolve));
          if (running()) {
            child.kill('SIGKILL');
            await bounded(closed, 'frm cleanup did not finish');
          }
          if (server) await server.close();
          await rm(root, { recursive: true, force: true });
        }
      });
      async function start() {
        await stop();
        starts += 1;
        output = '';
        child = spawn(process.execPath, [join(remoteRoot, 'bin/frm.js'), 'serve'],
          { cwd: root, env: { HOME: root, FRM_HOME: home, PATH: '' }, stdio: ['ignore', 'pipe', 'pipe'] });
        closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })));
        ready = new Promise((resolve, reject) => {
          child.stdout.on('data', data => {
            output += data;
            if (output.includes(`Local API: http://127.0.0.1:${port}/v1`)) resolve();
          });
          child.once('error', reject);
          child.once('close', () => reject(new Error('frm exited before readiness')));
        });
        ready.catch(() => {});
        child.stderr.on('data', data => { output += data; });
      }
      async function command(program, args) {
        if (program === 'loginctl') return { code: 0, stdout: 'yes\n' };
        assert.equal(program, 'systemctl');
        assert.equal(args[0], '--user');
        const action = args[1];
        if (action === 'is-enabled') return { code: enabled ? 0 : 1, stdout: '' };
        if (action === 'is-active') return { code: running() ? 0 : 3, stdout: '' };
        if (action === 'show') return { code: 0, stdout: running()
          ? `ActiveState=active\nSubState=running\nMainPID=${child.pid}\n`
          : 'ActiveState=inactive\nSubState=dead\nMainPID=0\n' };
        if (action === 'restart' || action === 'start') await start();
        else if (action === 'stop' || action === 'disable') await stop();
        else assert.ok(['enable', 'daemon-reload', 'show-environment'].includes(action), action);
        if (action === 'enable') enabled = true;
        if (action === 'disable') enabled = false;
        return { code: 0, stdout: '' };
      }
      childProcess.execFile = (program, args, options, callback) => {
        const commandChild = new EventEmitter();
        commandChild.kill = () => false;
        command(program, args).then(result => {
          callback(result.code ? new Error('inactive') : null, result.stdout);
          commandChild.emit('close', result.code, null);
        }, error => { callback(error, ''); commandChild.emit('close', 1, null); });
        return commandChild;
      };
      syncBuiltinESMExports();
      process.env = env;
      await mkdir(home, { recursive: true });
      const { createServer } = await fromPackage(serverRoot, 'dist/server.js');
      const { buildPairingUrl } = await fromPackage(serverRoot, 'dist/startup-access.js');
      const { runFrm } = await fromPackage(remoteRoot, 'dist/index.js');
      const requests = [];
      server = await createServer({ host: '127.0.0.1', port: 0,
        ...(token ? { serverToken: token } : {}) }, { provider: {
        async ready() {
          // Reserve through the initial readiness connection's port allocation.
          if (reservation.listening) await new Promise(resolve => reservation.close(resolve));
          return true;
        },
        async forward(request) {
          const chunks = [];
          for await (const chunk of request.body) chunks.push(chunk);
          requests.push({ path: request.path, headers: request.headers,
            body: JSON.parse(Buffer.concat(chunks).toString()) });
          return { status: 200, headers: { 'content-type': 'application/json' },
            body: [Buffer.from('{"id":"synthetic-result"}')] };
        },
      } });
      const address = await server.listen();
      context.diagnostic(JSON.stringify({ serverPort: address.port, localPort: port }));
      const origin = `http://127.0.0.1:${address.port}`;
      const link = buildPairingUrl(origin, token);
      const io = { stdout() {}, stderr(text) { assert.fail(text); } };
      const options = { program: [process.execPath, join(remoteRoot, 'bin/frm.js')] };
      const up = async () => {
        let summary = '';
        const code = await runFrm(['up', link, '--json'],
          { ...io, stdout(text) { summary += text; } }, options);
        assert.equal(code, 0);
        assert.equal(JSON.parse(summary).ready, true);
        assert.ok(running(), 'up must return with its managed child running');
        assert.equal((await exchange(port, '/ready', localToken)).status, 200,
          'up must wait for the local API before reporting success');
        return code;
      };
      assert.equal(await up(), 0);
      await bounded(ready, 'saved frm service did not become ready');
      assert.equal((await exchange(port, '/ready', localToken)).status, 200);
      for (const path of ['/v1/responses', '/v1/chat/completions']) {
        const body = { model: 'synthetic-model', stream: false, input: path };
        const result = await exchange(port, path, localToken, body);
        assert.deepEqual(result, { status: 200, body: '{"id":"synthetic-result"}' });
        assert.equal(requests.at(-1).path, path);
        assert.deepEqual(requests.at(-1).body, body);
      }
      if (localToken) assert.ok(!JSON.stringify(requests).includes(localToken));
      const profilePath = join(home, 'framerize-remote.json');
      const saved = await readFile(profilePath);
      const profile = JSON.parse(saved.toString());
      assert.equal(profile.remote, origin);
      assert.deepEqual(profile.serverToken, token ? { kind: 'value', value: token } : undefined);
      assert.equal(await up(), 0);
      assert.equal(starts, 1, 'unchanged setup must keep the same child');
      assert.deepEqual(await readFile(profilePath), saved);
      assert.equal(await runFrm(['service', 'stop'], io, options), 0);
      await assert.rejects(exchange(port, '/health', localToken), { code: 'ECONNREFUSED' });
      assert.equal(await up(), 0);
      await bounded(ready, 'saved frm service did not restart');
      assert.equal(starts, 2);
      assert.deepEqual(await readFile(profilePath), saved);
      assert.equal((await exchange(port, '/ready', localToken)).status, 200);
      assert.equal(await runFrm(['service', 'uninstall'], io, options), 0);
      await assert.rejects(exchange(port, '/health', localToken), { code: 'ECONNREFUSED' });
      assert.deepEqual(await readFile(profilePath), saved, 'uninstall preserves the profile');
      await server.close();
      await assert.rejects(exchange(address.port, '/health', token), { code: 'ECONNREFUSED' });
    });
}
