import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawn, spawnSync } from 'node:child_process';
import { constants } from 'node:fs';
import { mkdir, mkdtemp, open, readdir, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { connect } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageRoot = process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
const launcher = join(packageRoot, 'bin/frm.js');
const preload = fileURLToPath(new URL('./helpers/profile-up-preload.mjs', import.meta.url));
const args = ['--import', preload, launcher, 'up', '--json'];
const environment = root => ({ HOME: root, USERPROFILE: root, FRM_HOME: root,
  TMPDIR: root, TMP: root, TEMP: root,
  ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) });

async function runUp(root) {
  const child = spawn(process.execPath, args, { cwd: root, env: environment(root),
    stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '', stderr = '', error;
  child.stdout.setEncoding('utf8').on('data', text => { stdout += text; });
  child.stderr.setEncoding('utf8').on('data', text => { stderr += text; });
  child.on('error', value => { error = value; });
  const deadline = setTimeout(() => { error = new Error('frm up timed out'); child.kill('SIGKILL'); }, 3000);
  try {
    const closed = await new Promise(resolve => child.once('close', (status, signal) => resolve({ status, signal })));
    return { ...closed, stdout, stderr, error };
  } finally { clearTimeout(deadline); }
}

async function refused(port) {
  const socket = connect({ host: '127.0.0.1', port });
  try {
    const result = await new Promise(resolve => {
      socket.once('error', error => resolve(error.code));
      socket.once('connect', () => resolve('connected'));
      socket.setTimeout(1000, () => resolve('timeout'));
    });
    assert.equal(result, 'ECONNREFUSED');
  } finally { socket.destroy(); }
}

async function noFifoReader(path) {
  let handle;
  try {
    handle = await open(path, constants.O_WRONLY | constants.O_NONBLOCK);
    assert.fail('frm up left a FIFO reader open');
  } catch (error) { assert.equal(error.code, 'ENXIO'); }
  finally { await handle?.close(); }
}

function diagnostic(result, code) {
  assert.equal(result.error, undefined, 'frm up must finish before its deadline');
  assert.equal(result.signal, null);
  assert.equal(result.status, 1);
  assert.equal(result.stdout, '');
  assert.equal(JSON.parse(result.stderr).code, code);
}

for (const kind of ['fifo', 'missing', 'malformed', 'directory']) {
  test(`frm up rejects a ${kind} profile without changing private state`, {
    timeout: 8000, skip: kind === 'fifo' && process.platform === 'win32',
  }, async () => {
    const root = await mkdtemp(join(tmpdir(), 'frm-up-file-'));
    const profile = join(root, 'framerize-remote.json');
    let result;
    try {
      if (kind === 'fifo') execFileSync('mkfifo', ['-m', '600', profile], { timeout: 2000 });
      if (kind === 'malformed') await writeFile(profile, '{', { mode: 0o600 });
      if (kind === 'directory') await mkdir(profile);
      await writeFile(join(root, 'canary'), 'synthetic private state', { mode: 0o600 });
      const before = (await readdir(root)).sort();
      result = spawnSync(process.execPath, args, { cwd: root, env: environment(root),
        encoding: 'utf8', timeout: 3000, killSignal: 'SIGKILL', maxBuffer: 64 * 1024 });
      if (kind === 'fifo') await noFifoReader(profile);
      assert.deepEqual((await readdir(root)).sort(), before);
      assert.equal(await readFile(join(root, 'canary'), 'utf8'), 'synthetic private state');
      if (kind === 'malformed') assert.equal(await readFile(profile, 'utf8'), '{');
    } finally { await rm(root, { recursive: true, force: true }); }
    diagnostic(result, kind === 'missing' ? 'remote_required' : 'profile_invalid');
  });
}

for (const kind of ['regular', 'symlink']) {
  test(`frm up reads a ${kind} profile and restores exact bytes when readiness fails`, {
    timeout: 8000, skip: kind === 'symlink' && process.platform === 'win32',
  }, async () => {
    const root = await mkdtemp(join(tmpdir(), 'frm-up-rollback-'));
    const profile = join(root, 'framerize-remote.json');
    const target = join(root, 'saved.json');
    const requests = [];
    const server = createServer((request, response) => {
      requests.push([request.method, request.url]);
      request.resume();
      response.writeHead(503, { 'content-type': 'application/json', connection: 'close' });
      response.end(JSON.stringify({ apiVersion: 1, status: 'unavailable', reason: 'upstream_unavailable',
        capabilities: { responses: true, chatCompletions: true, streaming: true } }));
    });
    let result, port;
    try {
      await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
      port = server.address().port;
      const original = `{\n "remote": "http://127.0.0.1:${port}", "port": 19555\n}\n`;
      await writeFile(kind === 'symlink' ? target : profile, original, { mode: 0o600 });
      if (kind === 'symlink') await symlink(target, profile);
      result = await runUp(root);
      assert.equal(await readFile(profile, 'utf8'), original);
      if (kind === 'symlink') assert.equal(await readFile(target, 'utf8'), original);
      assert.deepEqual((await readdir(root)).sort(), kind === 'symlink'
        ? ['framerize-remote.json', 'saved.json'] : ['framerize-remote.json']);
    } finally {
      server.closeAllConnections();
      await new Promise(resolve => server.close(resolve));
      await rm(root, { recursive: true, force: true });
    }
    await refused(port);
    diagnostic(result, 'upstream_unavailable');
    assert.deepEqual(requests, [['GET', '/ready']]);
  });
}
