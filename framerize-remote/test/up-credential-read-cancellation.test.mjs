import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { connect } from 'node:net';
import fs from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const compiled = await build({ absWorkingDir: root, entryPoints: ['framerize-remote/src/cmdUp.ts'],
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
  alias: { '@framerize/core': join(root, 'framerize-core/src/index.ts') } });
const bytes = compiled.outputFiles[0].contents;
console.log(JSON.stringify({ bundleSha256: createHash('sha256').update(bytes).digest('hex'),
  inputs: Object.keys(compiled.metafile.inputs).sort() }));
const { setUp } = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

async function bounded(pending, label, milliseconds = 2000) {
  let timer;
  try { return await Promise.race([pending, new Promise((resolve, reject) => {
    timer = setTimeout(() => reject(new Error(`Timed out: ${label}`)), milliseconds);
  })]); } finally { clearTimeout(timer); }
}

async function within(pending, milliseconds) {
  let timer;
  try { return await Promise.race([pending.then(() => true), new Promise(resolve => {
    timer = setTimeout(() => resolve(false), milliseconds);
  })]); } finally { clearTimeout(timer); }
}

async function refused(port) {
  const socket = connect(port, '127.0.0.1');
  try {
    await bounded(new Promise((resolve, reject) => {
      socket.once('connect', () => reject(new Error('Fixture port remains open')));
      socket.once('error', error => error.code === 'ECONNREFUSED' ? resolve() : reject(error));
    }), 'fixture port refusal');
  } finally { socket.destroy(); }
}

async function fixture(context, key, hold) {
  const home = await fs.mkdtemp(join(tmpdir(), 'frm-credential-cancel-'));
  const configPath = join(home, 'framerize-remote.json');
  const tokenPath = join(home, 'synthetic-token');
  const state = { requests: [], commands: [], output: [], reads: 0, handles: new Set() };
  const controller = new AbortController();
  let entered;
  const reading = new Promise(resolve => { entered = resolve; });
  let release;
  const gate = new Promise(resolve => { release = resolve; });
  const originalOpen = fs.open;
  const sockets = new Set();
  const server = createServer((request, response) => {
    state.requests.push(request.url);
    response.writeHead(request.url === '/ready' ? 200 : 404, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ apiVersion: 1, status: 'ready',
      capabilities: { responses: true, chatCompletions: true, streaming: true } }));
  });
  server.on('connection', socket => {
    sockets.add(socket);
    socket.once('close', () => sockets.delete(socket));
  });
  let port;
  let cleaned = false;
  const cleanup = async () => {
    if (cleaned) return;
    release();
    controller.abort();
    try {
      if (state.pending) await bounded(state.pending, 'settle setup after release');
    } finally {
      fs.open = originalOpen;
      syncBuiltinESMExports();
      for (const handle of state.handles) await handle.close();
      for (const socket of sockets) socket.destroy();
      if (server.listening) await bounded(new Promise(resolve => server.close(resolve)), 'fixture close');
      if (port) await refused(port);
      await fs.rm(home, { recursive: true, force: true });
      cleaned = true;
    }
  };
  context.after(cleanup);
  await fs.writeFile(tokenPath, 'synthetic-credential\n', { mode: 0o600 });
  await bounded(new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', () => { server.off('error', reject); resolve(); });
  }), 'fixture listen');
  port = server.address().port;
  const original = `${JSON.stringify({ remote: `http://127.0.0.1:${port}`, host: '127.0.0.1', port: 8788,
    [key]: { kind: 'file', path: tokenPath } }, null, 3)}\n`;
  await fs.writeFile(configPath, original, { mode: 0o600 });
  fs.open = async (path, ...args) => {
    const handle = await originalOpen(path, ...args);
    if (path !== tokenPath) return handle;
    state.handles.add(handle);
    const read = handle.read.bind(handle);
    const close = handle.close.bind(handle);
    handle.read = async (...parameters) => {
      state.reads += 1;
      entered();
      if (hold) await gate;
      return read(...parameters);
    };
    handle.close = async () => {
      try { return await close(); } finally { state.handles.delete(handle); }
    };
    return handle;
  };
  syncBuiltinESMExports();
  const remote = `http://127.0.0.1:${port}`;
  const options = { configPath, signal: controller.signal,
    env: { HOME: home, FRM_HOME: home, XDG_CONFIG_HOME: join(home, 'xdg') },
    service: { host: { platform: 'freebsd', homeDir: home, uid: 501 }, home,
      program: [process.execPath, join(home, 'never-started.js'), 'serve'], environmentPath: '',
      run: async (...args) => { state.commands.push(args); throw new Error('Unexpected service command'); } } };
  const io = { stdout: text => state.output.push(text), stderr: text => state.output.push(text) };
  state.pending = setUp(remote, io, options).then(value => ({ value }), error => ({ error }));
  return { state, configPath, original, tokenPath, remote, controller, release, reading, cleanup };
}

for (const key of ['serverToken', 'localToken']) {
  test(`up cancellation restores the profile while the ${key} file read is pending`,
    { timeout: 8000 }, async context => {
      const fixtureState = await fixture(context, key, true);
      const { state, configPath, original, tokenPath, controller, reading, release, cleanup } = fixtureState;
      await bounded(reading, 'synthetic credential read');
      assert.notEqual(await fs.readFile(configPath, 'utf8'), original);
      controller.abort(new Error('synthetic setup cancellation'));
      const settledBeforeRelease = await within(state.pending, 1000);
      const restoredBeforeRelease = await fs.readFile(configPath, 'utf8') === original;
      release();
      const result = await bounded(state.pending, 'cancelled setup');
      assert.equal(result.error?.code, 'aborted');
      assert.equal(await fs.readFile(configPath, 'utf8'), original);
      assert.equal(await fs.readFile(tokenPath, 'utf8'), 'synthetic-credential\n');
      assert.ok(state.reads >= 1);
      assert.equal(state.handles.size, 0);
      assert.deepEqual(state.requests, []);
      assert.deepEqual(state.commands, []);
      assert.deepEqual(state.output, []);
      await cleanup();
      context.diagnostic(JSON.stringify({ key, settledBeforeRelease, restoredBeforeRelease,
        postReleaseCode: result.error.code, handlesClosed: true, portRefused: true,
        profileRestoredAfterRelease: true, tokenUnchanged: true, requests: state.requests.length }));
      assert.deepEqual({ settledBeforeRelease, restoredBeforeRelease },
        { settledBeforeRelease: true, restoredBeforeRelease: true });
    });
}

test('up reads a responsive credential file and reaches readiness', { timeout: 8000 }, async context => {
  const { state, configPath, original, tokenPath, remote, cleanup } = await fixture(context, 'serverToken', false);
  const result = await bounded(state.pending, 'responsive setup');
  assert.equal(result.error, undefined);
  assert.equal(result.value?.ready, true);
  assert.equal(result.value?.service, 'unavailable');
  assert.equal(result.value?.remote, remote);
  assert.notEqual(await fs.readFile(configPath, 'utf8'), original);
  assert.equal(JSON.parse(await fs.readFile(configPath, 'utf8')).remote, remote);
  assert.equal(await fs.readFile(tokenPath, 'utf8'), 'synthetic-credential\n');
  assert.ok(state.reads >= 1);
  assert.equal(state.handles.size, 0);
  assert.deepEqual(state.requests, ['/ready']);
  assert.deepEqual(state.commands, []);
  assert.deepEqual(state.output, []);
  await cleanup();
  context.diagnostic('Responsive file reached loopback readiness; handles closed and fixture port refused.');
});
