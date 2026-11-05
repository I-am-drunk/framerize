import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { getEventListeners } from 'node:events';
import { createServer } from 'node:http';
import { createConnection } from 'node:net';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({
  stdin: { contents: `export { offerClientSetup } from './src/clientCli.ts';
    export { nodeRunner } from './src/compatibility/process.ts';
    export { fetchText } from './src/compatibility/versions.ts';`,
    resolveDir: fileURLToPath(new URL('../', import.meta.url)) },
  bundle: true, platform: 'node', format: 'esm', write: false,
});
const { offerClientSetup, nodeRunner, fetchText } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function bounded(promise, milliseconds = 2000) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Operation did not settle')), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

function portResult(port) {
  return new Promise(resolve => {
    const socket = createConnection({ host: '127.0.0.1', port });
    socket.once('connect', () => { socket.destroy(); resolve('open'); });
    socket.once('error', error => { socket.destroy(); resolve(error.code); });
  });
}

function setupHarness(overrides = {}) {
  const controller = new AbortController(), output = [], questions = [];
  const io = { stdout: text => output.push(text), stderr: text => output.push(text),
    async ask(question) { questions.push(question); return 'yes'; } };
  const deps = { platform: 'linux', env: { PATH: '/synthetic/.local/bin' },
    home: '/synthetic', stateDir: '/synthetic/state', io, prompt: io.ask,
    target: { origin: 'http://127.0.0.1:8788' },
    probe: path => path === '/synthetic/.local/bin/claude', realPath: path => path,
    read: async () => null, fetch: async () => assert.fail('Unexpected release lookup'),
    runner: { capture: async () => '2.1.300', run: async () => assert.fail('Unexpected install or update') },
    ...overrides };
  return { controller, output, questions,
    start: () => offerClientSetup('/synthetic/profile.json', io, {}, deps, controller.signal) };
}

function assertCancelled(harness, result) {
  assert.equal(result, 'cancelled', 'The offer must finish promptly after cancellation');
  assert.equal(harness.questions.length, 1, 'No further client prompt may start');
  assert.deepEqual(harness.output, [], 'No version or launch message may follow cancellation');
  assert.equal(getEventListeners(harness.controller.signal, 'abort').length, 0);
}

async function releaseServer(t, handler) {
  const server = createServer(handler);
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const port = server.address().port;
  t.after(async () => {
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
    assert.equal(await portResult(port), 'ECONNREFUSED');
  });
  return { server, url: `http://127.0.0.1:${port}/release` };
}

for (const stage of ['headers', 'body']) {
  test(`cancelling the offer aborts a release fetch during ${stage}`, { timeout: 6000 }, async t => {
    const reached = Promise.withResolvers(), disconnected = Promise.withResolvers();
    const { server, url } = await releaseServer(t, (request, response) => {
      assert.equal(request.url, '/release');
      response.once('close', disconnected.resolve);
      if (stage === 'headers') reached.resolve();
      else { response.writeHead(200); response.flushHeaders(); }
    });
    const realFetch = globalThis.fetch;
    t.mock.method(globalThis, 'fetch', async (...args) => {
      const response = await realFetch(...args);
      if (stage === 'body') {
        const getReader = response.body.getReader.bind(response.body);
        t.mock.method(response.body, 'getReader', () => {
          const reader = getReader(), read = reader.read.bind(reader);
          t.mock.method(reader, 'read', () => {
            const pending = read();
            reached.resolve();
            return pending;
          });
          return reader;
        });
      }
      return response;
    });
    const h = setupHarness({ fetch: (endpoint, timeout, signal) => {
      assert.equal(endpoint, 'https://downloads.claude.ai/claude-code-releases/latest');
      return fetchText(url, timeout, signal);
    } });
    const pending = h.start();
    let result, released = false;
    try {
      await bounded(reached.promise);
      h.controller.abort();
      result = await bounded(pending, 750).catch(() => 'pending');
      if (result === 'cancelled') {
        released = await bounded(disconnected.promise, 750).then(() => true, () => false);
      }
      t.diagnostic(JSON.stringify({ stage, result, released }));
    } finally {
      server.closeAllConnections();
      await bounded(pending);
    }
    assertCancelled(h, result);
    assert.equal(released, true, 'Cancellation must close the pending HTTP response');
  });
}

test('a live caller signal preserves the release-body deadline', { timeout: 4000 }, async t => {
  const disconnected = Promise.withResolvers();
  const { url } = await releaseServer(t, (_request, response) => {
    response.once('close', disconnected.resolve);
    response.writeHead(200);
    response.flushHeaders();
  });
  const controller = new AbortController();
  assert.equal(await bounded(fetchText(url, 100, controller.signal)), null);
  await bounded(disconnected.promise);
  assert.equal(controller.signal.aborted, false);
});

function probeFixture(t, tree) {
  const reached = Promise.withResolvers();
  let child, closed, exited, port, finished = false;
  const server = `const server = require('node:net').createServer(socket => socket.end());
    server.listen(0, '127.0.0.1', () => {
      console.log(JSON.stringify({port: server.address().port})); process.send?.('ready');
    });`;
  const program = tree ? `const child = require('node:child_process').spawn(process.execPath,
    ['-e', ${JSON.stringify(server)}], {stdio: ['ignore', 'inherit', 'ignore', 'ipc']});
    child.once('message', () => process.exit(0));` : server;
  const runner = nodeRunner(process.platform, (_file, _args, options) => {
    child = spawn(process.execPath, ['-e', program], options);
    exited = new Promise(resolve => child.once('exit', resolve));
    closed = new Promise(resolve => child.once('close', () => { finished = true; resolve(); }));
    let output = '';
    child.stdout.on('data', chunk => {
      output += chunk;
      if (output.includes('\n')) { port = JSON.parse(output.trim()).port; reached.resolve(port); }
    });
    child.once('error', reached.reject);
    return child;
  });
  const cleanup = async () => {
    if (child && !finished) {
      if (process.platform === 'win32') child.kill('SIGKILL');
      else { try { process.kill(-child.pid, 'SIGKILL'); } catch {} }
    }
    if (closed) await bounded(closed);
    if (port) assert.equal(await portResult(port), 'ECONNREFUSED');
  };
  t.after(cleanup);
  return { runner, reached: reached.promise, cleanup, waitClose: () => closed, waitExit: () => exited };
}

for (const stage of ['installed', 'latest']) {
  test(`cancelling the offer stops the ${stage} version probe`, {
    timeout: 8000, skip: stage === 'latest' && process.platform === 'win32'
      ? 'POSIX descendant process-group cleanup' : false,
  }, async t => {
    const probe = probeFixture(t, stage === 'latest');
    const commands = [];
    const h = setupHarness({
      probe: path => ['/synthetic/.local/bin/claude', '/synthetic/.local/bin/npm'].includes(path),
      realPath: () => '/synthetic/lib/node_modules/@anthropic-ai/claude-code/cli.js',
      runner: { run: async () => assert.fail('Unexpected install or update'),
        capture(command, env, signal) {
          commands.push(command.args[0]);
          if (stage === 'latest' && command.args[0] === '--version') return Promise.resolve('2.1.300');
          return probe.runner.capture(command, env, signal);
        } },
    });
    const pending = h.start();
    let result, released = false;
    try {
      const port = await bounded(probe.reached);
      if (stage === 'latest') await bounded(probe.waitExit());
      h.controller.abort();
      result = await bounded(pending, 750).catch(() => 'pending');
      if (result === 'cancelled') {
        released = await bounded(probe.waitClose(), 750).then(() => true, () => false);
        if (released) assert.equal(await portResult(port), 'ECONNREFUSED');
      }
      t.diagnostic(JSON.stringify({ stage, result, released }));
    } finally { await probe.cleanup(); await bounded(pending); }
    assertCancelled(h, result);
    assert.equal(released, true, 'Cancellation must stop the captured process tree');
    assert.deepEqual(commands, stage === 'installed' ? ['--version'] : ['--version', 'view']);
  });
}

test('already-cancelled operations do not start a fetch or probe', async t => {
  const controller = new AbortController();
  controller.abort();
  let fetches = 0, spawns = 0;
  t.mock.method(globalThis, 'fetch', async () => { fetches++; return new Response('2.1.300'); });
  const runner = nodeRunner(process.platform, () => { spawns++; throw new Error('Unexpected spawn'); });
  const fetched = await fetchText('https://release.invalid/latest', 1000, controller.signal);
  const captured = await runner.capture({ command: 'unused', args: [], display: 'unused' }, {}, controller.signal);
  assert.deepEqual({ fetched, captured, fetches, spawns }, { fetched: null, captured: null, fetches: 0, spawns: 0 });
});

for (const stage of ['installed version', 'channel settings']) {
  test(`cancellation after ${stage} prevents the release lookup and output`, async () => {
    let h, fetches = 0;
    h = setupHarness({
      fetch: async () => { fetches++; return '2.1.300'; },
      read: async () => { if (stage === 'channel settings') h.controller.abort(); return null; },
      runner: { run: async () => assert.fail('Unexpected command'),
        capture: async () => { if (stage === 'installed version') h.controller.abort(); return '2.1.300'; } },
    });
    assertCancelled(h, await h.start());
    assert.equal(fetches, 0);
  });
}

test('a successful probe removes its cancellation listener', async () => {
  const controller = new AbortController();
  const output = await nodeRunner().capture({ command: process.execPath,
    args: ['-e', 'console.log("2.1.300")'], display: 'synthetic version' }, {}, controller.signal);
  assert.equal(output, '2.1.300\n');
  assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
  controller.abort();
});
