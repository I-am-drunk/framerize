import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { once } from 'node:events';
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { connect } from 'node:net';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const source = name => JSON.stringify(fileURLToPath(new URL(`../src/${name}.ts`, import.meta.url)));
const compiled = await build({
  stdin: { contents: ['cmdUp', 'configStore', 'serviceUnits', 'serviceManager', 'diagnostics']
    .map(name => `export * from ${source(name)};`).join('\n'),
    resolveDir: fileURLToPath(new URL('.', import.meta.url)) },
  alias: { '@framerize/core': fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)) },
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
});
const digest = value => createHash('sha256').update(value).digest('hex');
const api = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
console.log(JSON.stringify({ node: process.version, bundle: digest(compiled.outputFiles[0].contents),
  inputs: Object.fromEntries(await Promise.all(Object.keys(compiled.metafile.inputs)
    .filter(path => path !== '<stdin>').map(async path => [path, digest(await readFile(path))]))) }));

async function bounded(promise, milliseconds = 4000) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error('fixture exceeded its bound')), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function fixture(context) {
  const root = await mkdtemp(join(tmpdir(), 'frm-readiness-query-'));
  const server = createServer((request, response) => {
    response.writeHead(200, { 'content-type': 'application/json', 'x-framerize-remote-pid': '4242' });
    response.end(JSON.stringify({ apiVersion: 1, status: 'ready',
      capabilities: { responses: true, chatCompletions: true, streaming: true } }));
  });
  let port;
  context.after(async () => {
    try {
      server.closeAllConnections();
      await bounded(new Promise(resolve => server.close(resolve)));
      if (port !== undefined) {
        const socket = connect({ host: '127.0.0.1', port });
        try {
          const result = await bounded(new Promise(resolve => {
            socket.once('error', error => resolve(error.code));
            socket.once('connect', () => resolve('connected'));
          }));
          assert.equal(result, 'ECONNREFUSED');
        } finally { socket.destroy(); }
      }
    } finally { await rm(root, { recursive: true, force: true }); }
  });
  server.listen(0, '127.0.0.1');
  await bounded(once(server, 'listening'));
  port = server.address().port;
  return { root, port };
}

for (const mode of ['deadline', 'caller cancellation', 'cleanup failure']) {
  test(`setup readiness propagates ${mode} and waits for query cleanup`, { timeout: 10000 }, async context => {
    const controller = new AbortController();
    const entered = Promise.withResolvers(), aborted = Promise.withResolvers(), release = Promise.withResolvers();
    const records = [], counts = new Map();
    const failure = new api.ServiceCommandCleanupError(new Error('synthetic denied cleanup'));
    let pending, settled = false, abortedCount = 0;
    context.after(async () => {
      controller.abort();
      release.resolve();
      for (const record of records) record.finish();
      if (pending) await bounded(pending);
    });
    const peer = await fixture(context);
    const home = join(peer.root, 'home'), configPath = join(home, 'framerize-remote.json');
    const service = { host: { platform: 'linux', homeDir: peer.root, uid: 1000 },
      home, program: [process.execPath, 'synthetic-frm.mjs', 'serve'], environmentPath: '/synthetic/bin' };
    const manager = api.selectServiceManager(service.host);
    const unit = api.renderServiceUnit({ ...service, logPath: join(home, 'frm-service.log') });
    await mkdir(dirname(manager.unitPath), { recursive: true });
    await writeFile(manager.unitPath, unit);
    await api.saveConfigFile(configPath, { remote: `http://127.0.0.1:${peer.port}`, host: '127.0.0.1',
      port: peer.port, allowInsecureRemote: true });
    const before = await readFile(configPath);
    service.run = (command, args, signal) => {
      const kind = args.includes('--property=MainPID') ? 'pid'
        : args.includes('is-active') ? 'active' : args.includes('is-enabled') ? 'enabled' : undefined;
      if (!kind) {
        assert.ok(args.includes('show-environment') || command === 'loginctl');
        return Promise.resolve({ code: 0, stdout: command === 'loginctl' ? 'yes\n' : '' });
      }
      counts.set(kind, (counts.get(kind) ?? 0) + 1);
      if (kind !== 'pid' && counts.get(kind) === 1) return Promise.resolve({ code: 0, stdout: '' });
      return new Promise((resolve, reject) => {
        const record = { kind, signal, done: false, finish: undefined };
        const finish = () => {
          if (record.done) return;
          record.done = true;
          signal?.removeEventListener('abort', abort);
          if (mode === 'cleanup failure' && kind === 'pid') reject(failure);
          else resolve({ code: 1, stdout: '' });
        };
        const abort = () => {
          if (++abortedCount === 3) aborted.resolve();
          void release.promise.then(finish);
        };
        record.finish = finish;
        records.push(record);
        if (records.length === 3) entered.resolve();
        if (signal?.aborted) abort();
        else signal?.addEventListener('abort', abort, { once: true });
      });
    };
    const output = [];
    const io = { stdout: text => output.push(text), stderr: text => output.push(text) };
    pending = api.setUp(undefined, io, { configPath, service, env: {},
      readyTimeoutMs: mode === 'deadline' ? 2000 : 8000, signal: controller.signal })
      .then(value => { settled = true; return { value }; }, error => { settled = true; return { error }; });
    await bounded(entered.promise);
    assert.deepEqual(records.map(record => record.kind).sort(), ['active', 'enabled', 'pid']);
    assert.ok(records.every(record => record.signal === records[0].signal));
    assert.notEqual(records[0].signal, controller.signal);
    if (mode !== 'deadline') controller.abort();
    await bounded(aborted.promise);
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(settled, false, 'setup must await query cleanup');
    assert.ok(records.every(record => !record.done));
    release.resolve();
    const result = await bounded(pending);
    assert.ok(records.every(record => record.done));
    if (mode === 'cleanup failure') assert.equal(result.error, failure);
    else assert.equal(result.error?.code, mode === 'deadline' ? 'service_not_ready' : 'aborted');
    assert.deepEqual(await readFile(configPath), before);
    assert.equal(await readFile(manager.unitPath, 'utf8'), unit);
    assert.deepEqual(output, []);
    context.diagnostic(JSON.stringify({ mode, queries: records.length,
      cleanupBeforeSettlement: true, privateSettingsUnchanged: true }));
  });
}
