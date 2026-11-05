import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import { createServer, get } from 'node:http';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

async function bounded(pending, message) {
  let timer;
  try {
    return await Promise.race([pending, new Promise((resolve, reject) => {
      timer = setTimeout(() => reject(new Error(message)), 5000);
    })]);
  } finally { clearTimeout(timer); }
}

function health(port) {
  return new Promise((resolve, reject) => {
    const request = get({ host: '127.0.0.1', port, path: '/health', agent: false,
      signal: AbortSignal.timeout(2000) }, response => {
      response.on('error', reject);
      response.on('end', () => resolve(response.statusCode));
      response.resume();
    });
    request.on('error', reject);
  });
}

test('frm serves from environment without tokens or a saved profile',
  { timeout: 20_000 }, async context => {
    const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
      ?? fileURLToPath(new URL('../', import.meta.url)));
    const manifest = JSON.parse(await readFile(join(packageRoot, 'package.json'), 'utf8'));
    assert.equal(typeof manifest.bin?.frm, 'string', 'package.json must declare the canonical bin.frm');
    assert.ok(manifest.bin.frm.length > 0, 'bin.frm must name a real launcher');
    const launcher = resolve(packageRoot, manifest.bin.frm);
    const root = await mkdtemp(join(tmpdir(), 'remote-frm-launcher-'));
    const calls = [];
    const remote = createServer((request, response) => {
      calls.push({ method: request.method, path: request.url,
        authorization: request.headers.authorization });
      request.resume();
      response.setHeader('content-type', 'application/json');
      response.statusCode = request.url === '/ready' ? 200 : 404;
      response.end(JSON.stringify(request.url === '/ready'
        ? { apiVersion: 1, status: 'ready',
          capabilities: { responses: true, chatCompletions: true, streaming: true } }
        : { error: 'unexpected_fixture_route' }));
    });
    let child;
    let closed;
    let lines;
    context.after(async () => {
      try {
        if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
        if (closed) await bounded(closed, 'frm child cleanup timed out');
      } finally {
        lines?.close();
        remote.closeAllConnections();
        await new Promise(resolve => remote.close(resolve));
        await rm(root, { recursive: true, force: true });
      }
    });
    assert.deepEqual(await readdir(root), []);
    const opened = once(remote, 'listening');
    remote.listen(0, '127.0.0.1');
    await bounded(opened, 'Synthetic remote did not start');
    child = spawn(process.execPath, [launcher, 'serve', '--json'], {
      cwd: root, stdio: ['ignore', 'pipe', 'pipe'],
      env: { HOME: root, USERPROFILE: root, XDG_CONFIG_HOME: root,
        TMPDIR: root, TEMP: root, TMP: root, NODE_DISABLE_COMPILE_CACHE: '1',
        FRM_REMOTE: `http://127.0.0.1:${remote.address().port}`,
        FRM_HOME: join(root, 'profile'), FRM_PORT: '0',
        ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}),
      },
    });
    closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })));
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8').on('data', chunk => { stdout += chunk; });
    child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk; });
    lines = createInterface({ input: child.stdout });
    const listening = new Promise((resolve, reject) => {
      child.once('error', () => reject(new Error('frm launcher could not start')));
      child.once('close', (code, signal) => reject(new Error(
        `frm exited before ready: code=${code}, signal=${signal}`)));
      lines.on('line', line => {
        let record;
        try { record = JSON.parse(line); }
        catch { reject(new Error('frm output must be JSON')); return; }
        if (record?.listening) resolve(record);
      });
    });
    const ready = await bounded(listening, 'frm did not report ready');
    const port = ready.listening.port;
    assert.ok(Number.isInteger(port) && port > 0 && port <= 65535, 'Expected an assigned port');
    assert.deepEqual(ready, { ready: true, listening: { host: '127.0.0.1', port } });
    assert.notEqual(port, remote.address().port);
    assert.equal(await health(port), 200);
    assert.equal(child.exitCode, null);
    assert.equal(child.signalCode, null);
    const assertNoFiles = async () => {
      const entries = await readdir(root, { recursive: true, withFileTypes: true });
      assert.ok(entries.every(entry => entry.isDirectory()),
        'Environment-only serve must not write profile, token or other state files');
    };
    await assertNoFiles();
    assert.equal(child.kill('SIGTERM'), true);
    const result = await bounded(closed, 'frm did not stop after SIGTERM');
    if (process.platform === 'win32') assert.ok(result.code === 0 || result.signal === 'SIGTERM');
    else assert.deepEqual(result, { code: 0, signal: null });
    await assert.rejects(health(port), { code: 'ECONNREFUSED' });
    await assertNoFiles();
    assert.equal(stderr, '');
    const events = stdout.trim().split('\n').map(line => JSON.parse(line));
    assert.deepEqual(events[0], ready);
    if (process.platform !== 'win32') assert.deepEqual(events, [ready, { stopped: true }]);
    assert.deepEqual(calls, [{ method: 'GET', path: '/ready', authorization: undefined }]);
    context.diagnostic('Canonical frm reached /health 200, stopped and released its port without saved files.');
  });
