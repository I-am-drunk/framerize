import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { get } from 'node:http';
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

test('real CLI checks token before selection and serves after repairing the account list',
  { timeout: 30_000 }, async context => {
    const root = mkdtempSync(join(tmpdir(), 'server-selection-recovery-'));
    let child;
    let closed;
    let lines;
    context.after(async () => {
      try {
        if (child && child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
        if (closed) await bounded(closed, 'CLI cleanup timed out');
      } finally {
        lines?.close();
        rmSync(root, { recursive: true, force: true });
      }
    });
    const tokenFile = join(root, 'private-token-path-canary');
    const accountsFile = join(root, 'private-account-path-canary');
    const token = 'synthetic-recovery-token-canary';
    const account = { id: 'first-account-canary', projectId: 'project-canary',
      session: { env: 'RECOVERY_SESSION_CANARY' } };
    const args = [launcher, 'serve', '--backend', 'framer', '--host', '127.0.0.1',
      '--port', '0', '--server-token-file', tokenFile, '--accounts-file', accountsFile,
      '--framer-origin', 'http://127.0.0.1:1'];
    const options = { cwd: root, env: {
      HOME: root, USERPROFILE: root, TMPDIR: root, TMP: root, TEMP: root,
      RECOVERY_SESSION_CANARY: 'session=synthetic-session-canary',
      NODE_DISABLE_COMPILE_CACHE: '1',
      ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}),
    } };
    const assertRedacted = output => {
      assert.ok(!/canary/i.test(output), 'CLI must not log synthetic secrets or account metadata');
      assert.ok(!output.includes(root), 'CLI must not log the private state path');
    };
    const startupFailure = () => {
      const result = spawnSync(process.execPath, args, {
        ...options, encoding: 'utf8', timeout: 5000, killSignal: 'SIGKILL',
      });
      assert.ok(!result.error, 'Failed startup must complete within five seconds');
      assert.equal(result.status, 1);
      assert.equal(result.signal, null);
      assertRedacted(result.stdout + result.stderr);
      assert.ok(result.stdout === '', 'Failed startup must not write stdout');
      assert.ok(!result.stderr.includes('server.listening'), 'Invalid setup must prevent listening');
      let diagnostic;
      try { diagnostic = JSON.parse(result.stderr); }
      catch { assert.fail('Failed startup must report a JSON diagnostic'); }
      assert.ok(diagnostic.event === 'server.failed', 'Expected a startup failure diagnostic');
      return diagnostic;
    };
    writeFileSync(tokenFile, `${token} invalid`, { mode: 0o600 });
    writeFileSync(accountsFile, JSON.stringify([account,
      { ...account, id: 'second-account-canary' }]), { mode: 0o600 });
    const tokenFailure = startupFailure();
    writeFileSync(tokenFile, token, { mode: 0o600 });
    const selectionFailure = startupFailure();
    assert.equal(selectionFailure.message, 'Select one configured Framer account.');
    assert.equal(selectionFailure.next,
      'Check --account-id and --accounts-file. Select one enabled account with projectId and a session reference.');
    writeFileSync(accountsFile, JSON.stringify([account]), { mode: 0o600 });
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
    const port = await bounded(listening, 'Recovered CLI did not report a listener');
    assert.ok(Number.isInteger(port) && port > 0 && port <= 65535, 'Expected an assigned port');
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
    context.diagnostic('Account repair reached /health 200, SIGTERM close and ECONNREFUSED.');
    assert.equal(tokenFailure.message, 'Server token file is unavailable or invalid.');
    assert.equal(tokenFailure.next,
      'Check --server-token-file, its contents and permissions. On Unix, use chmod 600.');
  });
