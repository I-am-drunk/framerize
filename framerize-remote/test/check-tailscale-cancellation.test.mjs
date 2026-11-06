import assert from 'node:assert/strict';
import processes from 'node:child_process';
import { createHash } from 'node:crypto';
import { once } from 'node:events';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { syncBuiltinESMExports } from 'node:module';
import { createConnection } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { build } from 'esbuild';

const sourceRoot = resolve(process.env.FRAMERIZE_REMOTE_SOURCE_ROOT
  ?? fileURLToPath(new URL('../src/', import.meta.url)));
const compiled = await build({ absWorkingDir: sourceRoot, entryPoints: ['cmdRun.ts'],
  bundle: true, write: false, metafile: true, platform: 'node', format: 'esm',
  alias: { '@framerize/core': resolve(sourceRoot, '../../framerize-core/src/index.ts') } });
const bytes = compiled.outputFiles[0].contents;
const { runCheck } = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
const hash = value => createHash('sha256').update(value).digest('hex');
const inputs = await Promise.all(Object.keys(compiled.metafile.inputs).sort().map(async path =>
  ({ path, sha256: hash(await readFile(resolve(sourceRoot, path))) })));
console.log(JSON.stringify({ runtime: process.version, bundle: hash(bytes), inputs }));

async function bounded(promise, label, milliseconds = 2500) {
  let timer;
  try { return await Promise.race([promise, new Promise((resolveUnused, reject) => {
    timer = setTimeout(() => reject(new Error(label)), milliseconds);
  })]); } finally { clearTimeout(timer); }
}

async function contact(port, message = 'ping') {
  const socket = createConnection({ host: '127.0.0.1', port });
  try {
    await bounded(once(socket, 'connect'), 'Fixture connection timed out');
    const ended = once(socket, 'end');
    socket.resume();
    socket.end(message);
    await bounded(ended, 'Fixture response timed out');
  } finally { socket.destroy(); }
}

const worker = `
const { createServer } = require('node:net');
const server = createServer(socket => socket.once('data', bytes => {
  socket.end('owned fixture');
  if (bytes.toString() === 'stop') server.close();
}));
server.listen(0, '127.0.0.1', () => process.stdout.write(JSON.stringify({
  BackendState: 'Running', Self: { TailscaleIPs: ['100.64.0.1'] },
  fixture: { pid: process.pid, port: server.address().port }
}) + '\\n'));
`;

for (const mode of ['json cancellation', 'human cancellation', 'success control']) {
  test(`check with Tailscale: ${mode}`, { timeout: 8000 }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'frm-check-tailnet-'));
    const profile = join(root, 'profile.json');
    const controller = new AbortController();
    const connections = new Set(), requests = [], commands = [];
    let child, closed, operation, port, childOutput = '';
    const upstream = createServer((request, response) => {
      requests.push(request.url);
      request.resume();
      response.setHeader('content-type', 'application/json');
      response.end(JSON.stringify({ apiVersion: 1, status: 'ready',
        capabilities: { responses: true, chatCompletions: true, streaming: true } }));
    });
    upstream.on('connection', socket => {
      connections.add(socket);
      socket.once('close', () => connections.delete(socket));
    });
    context.after(async () => {
      controller.abort();
      try {
        if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
        if (closed) await bounded(closed, 'Owned child cleanup timed out');
        if (operation) await bounded(operation, 'Check cleanup timed out');
        if (port) await assert.rejects(contact(port), { code: 'ECONNREFUSED' });
      } finally {
        context.mock.restoreAll();
        syncBuiltinESMExports();
        for (const socket of connections) socket.destroy();
        await new Promise(resolve => upstream.close(resolve));
        await rm(root, { recursive: true, force: true });
      }
    });
    upstream.listen(0, '127.0.0.1');
    await bounded(once(upstream, 'listening'), 'Upstream did not listen');
    const origin = `http://127.0.0.1:${upstream.address().port}`;
    const saved = `${JSON.stringify({ remote: origin })}\n`;
    await writeFile(profile, saved, { mode: 0o600 });
    const entered = Promise.withResolvers();
    const spawn = processes.spawn;
    context.mock.method(processes, 'spawn', (command, args, options) => {
      commands.push([command, args]);
      assert.equal(command, process.platform === 'win32' ? 'tailscale.exe' : 'tailscale');
      assert.deepEqual(args, ['status', '--json']);
      assert.equal(options.shell, false);
      child = spawn(process.execPath, ['-e', worker], options);
      closed = once(child, 'close');
      child.once('error', entered.reject);
      child.stdout.on('data', chunk => {
        childOutput += chunk;
        if (!childOutput.endsWith('\n')) return;
        const fixture = JSON.parse(childOutput).fixture;
        assert.equal(fixture.pid, child.pid);
        port = fixture.port;
        entered.resolve();
      });
      return child;
    });
    syncBuiltinESMExports();
    const json = mode !== 'human cancellation';
    operation = runCheck(profile, json, { PATH: '', HOME: root }, true, controller.signal)
      .then(result => ({ result }), error => ({ error: { code: error.code, message: error.message } }));
    await bounded(Promise.race([entered.promise, operation.then(result => {
      throw new Error(`Check ended before synthetic Tailscale: ${JSON.stringify(result)}`);
    })]), 'Synthetic Tailscale did not start');
    await contact(port);
    const cancelled = mode !== 'success control';
    if (cancelled) controller.abort();
    else await contact(port, 'stop');
    const outcome = await bounded(operation, 'Check ignored cancellation');
    const childExit = await bounded(closed, 'Synthetic Tailscale remained alive');
    await assert.rejects(contact(port), { code: 'ECONNREFUSED' });
    assert.equal(await readFile(profile, 'utf8'), saved);
    assert.deepEqual(requests, ['/ready']);
    assert.equal(commands.length, 1);
    for (const socket of connections) socket.destroy();
    const upstreamPort = upstream.address().port;
    await new Promise(resolve => upstream.close(resolve));
    await assert.rejects(contact(upstreamPort), { code: 'ECONNREFUSED' });
    await rm(root, { recursive: true, force: true });
    await assert.rejects(readFile(profile), { code: 'ENOENT' });
    context.diagnostic(JSON.stringify({ mode, outcome, childExit,
      childPortClosed: true, upstreamPortClosed: true, profileUnchanged: true, privateStateRemoved: true }));
    assert.equal(outcome.error, undefined);
    assert.equal(outcome.result.exitCode, cancelled ? 1 : 0);
    if (json) {
      const result = JSON.parse(outcome.result.text);
      assert.equal(result.ready, !cancelled, 'A cancelled check must not announce readiness success');
      if (cancelled) assert.equal(result.diagnostic.code, 'aborted');
      else assert.equal(result.tailscale.state, 'running');
    } else {
      assert.doesNotMatch(outcome.result.text, /Remote readiness passed/);
      assert.match(outcome.result.text, /The command was cancelled\./);
    }
  });
}
