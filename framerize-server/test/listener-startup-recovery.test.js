import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { createServer, get } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline';
import { fileURLToPath } from 'node:url';

const launcher = fileURLToPath(new URL('../bin/framerize-server.js', import.meta.url));

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

test('real CLI recovers after its occupied listener port is released',
  { timeout: 30_000 }, async context => {
    const root = mkdtempSync(join(tmpdir(), 'server-listener-recovery-'));
    const occupied = createServer((request, response) => {
      response.writeHead(418);
      response.end('occupied fixture');
    });
    let child;
    let closed;
    let lines;
    context.after(async () => {
      try {
        if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
        if (closed) await bounded(closed, 'CLI cleanup timed out');
      } finally {
        try {
          lines?.close();
          occupied.closeAllConnections();
          if (occupied.listening) await bounded(new Promise(resolve => occupied.close(resolve)),
            'Fixture cleanup timed out');
        } finally { rmSync(root, { recursive: true, force: true }); }
      }
    });
    const tokenFile = join(root, 'private-token-path-canary');
    const accountsFile = join(root, 'private-account-path-canary');
    writeFileSync(tokenFile, 'synthetic-listener-token-canary', { mode: 0o600 });
    writeFileSync(accountsFile, '[]', { mode: 0o600 });
    await bounded(new Promise((resolve, reject) => {
      occupied.once('error', reject);
      occupied.listen(0, '127.0.0.1', resolve);
    }), 'Fixture did not start');
    const port = occupied.address().port;
    const args = [launcher, 'serve', '--backend', 'http', '--host', '127.0.0.1',
      '--port', String(port), '--server-token-file', tokenFile, '--accounts-file', accountsFile];
    const options = { cwd: root, env: {
      HOME: root, USERPROFILE: root, TMPDIR: root, TMP: root, TEMP: root,
      NODE_DISABLE_COMPILE_CACHE: '1',
      ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}),
    } };
    const assertRedacted = output => {
      assert.ok(!/canary/i.test(output), 'CLI must not log synthetic credentials or private filenames');
      assert.ok(!output.includes(root), 'CLI must not log the private state path');
    };
    const failed = spawnSync(process.execPath, args, {
      ...options, encoding: 'utf8', timeout: 5000, killSignal: 'SIGKILL',
    });
    assert.ok(!failed.error, 'Failed startup must finish without a spawn error or timeout');
    assert.equal(failed.status, 1);
    assert.equal(failed.signal, null);
    assertRedacted(failed.stdout + failed.stderr);
    assert.ok(failed.stdout === '', 'Failed startup must not write stdout');
    assert.ok(!failed.stderr.includes('server.listening'), 'Occupied port must prevent listening');
    let diagnostic;
    try { diagnostic = JSON.parse(failed.stderr); }
    catch { assert.fail('Failed startup must report a JSON diagnostic'); }
    assert.ok(diagnostic.event === 'server.failed', 'Expected a startup failure diagnostic');
    assert.equal(occupied.listening, true);
    assert.equal(await health(port), 418);
    await bounded(new Promise((resolve, reject) => occupied.close(error => {
      if (error) reject(error);
      else resolve();
    })), 'Fixture did not release its port');
    assert.equal(occupied.listening, false);
    child = spawn(process.execPath, args, { ...options, stdio: ['ignore', 'pipe', 'pipe'] });
    closed = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })));
    let output = '';
    child.stdout.setEncoding('utf8').on('data', chunk => { output += chunk; });
    child.stderr.setEncoding('utf8').on('data', chunk => { output += chunk; });
    lines = createInterface({ input: child.stderr });
    const listening = new Promise((resolve, reject) => {
      child.once('error', () => reject(new Error('CLI could not start')));
      child.once('close', () => reject(new Error('CLI exited before listening')));
      lines.on('line', line => {
        let record;
        try { record = JSON.parse(line); }
        catch { reject(new Error('CLI log was not JSON')); return; }
        if (record.event === 'server.listening') resolve(record.port);
      });
    });
    assert.equal(await bounded(listening, 'Recovered CLI did not report a listener'), port);
    assert.equal(await health(port), 200);
    assert.equal(child.exitCode, null);
    assert.equal(child.signalCode, null);
    assert.equal(child.kill('SIGTERM'), true);
    const result = await bounded(closed, 'Recovered CLI did not stop');
    if (process.platform === 'win32') assert.ok(result.code === 0 || result.signal === 'SIGTERM');
    else assert.deepEqual(result, { code: 0, signal: null });
    await assert.rejects(health(port), { code: 'ECONNREFUSED' });
    assertRedacted(output);
    assert.ok(!output.includes('server.failed'), 'Recovered CLI must not report a failure');
    context.diagnostic('Original listener survived; recovery reached /health 200, SIGTERM close and ECONNREFUSED.');
    assert.deepEqual(diagnostic, {
      event: 'server.failed',
      message: 'Server could not listen on the configured host and port.',
      next: 'Check --host or FRAMERIZE_SERVER_HOST and --port or FRAMERIZE_SERVER_PORT. Use an available local address and port.',
    });
  });
