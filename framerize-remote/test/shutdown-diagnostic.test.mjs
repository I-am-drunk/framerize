import test from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, writeFile, rm } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const launcher = fileURLToPath(new URL('../bin/framerize-remote.js', import.meta.url));
const failureHook = `
import { Server } from 'node:http';
const close = Server.prototype.close;
Server.prototype.close = function(callback) {
  close.call(this, callback);
  throw new Error('private-close-canary');
};
if (process.env.REMOTE_TEST_PHASE === 'startup') {
  const listen = Server.prototype.listen;
  Server.prototype.listen = function(...args) {
    this.once('listening', () => process.emit('SIGTERM'));
    return listen.apply(this, args);
  };
} else {
  const write = process.stdout.write;
  process.stdout.write = function(chunk, ...args) {
    const result = write.call(this, chunk, ...args);
    if (/Local API:|"listening"/.test(String(chunk))) process.emit('SIGTERM');
    return result;
  };
}
`;

for (const phase of ['startup', 'serving']) for (const json of [false, true]) {
  test(`launcher reports failed cleanup during ${phase} with ${json ? 'JSON' : 'compact'} output`,
    { timeout: 7000 }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'remote-cleanup-diagnostic-'));
    const remote = createServer((request, response) => {
      request.resume();
      response.end(JSON.stringify({ apiVersion: 1, status: 'ready',
        capabilities: { responses: true, chatCompletions: true, streaming: true } }));
    });
    let child;
    context.after(async () => {
      if (child && child.exitCode === null && child.signalCode === null) {
        child.kill('SIGKILL'); await once(child, 'close');
      }
      remote.closeAllConnections();
      await new Promise(resolve => remote.close(resolve));
      await rm(root, { recursive: true, force: true });
    });
    remote.listen(0, '127.0.0.1'); await once(remote, 'listening');
    const profile = join(root, 'profile.json');
    const hook = join(root, 'failure-hook.mjs');
    const serverToken = join(root, 'server-token');
    const localToken = join(root, 'local-token');
    await writeFile(serverToken, 'synthetic-server-token');
    await writeFile(localToken, 'synthetic-local-token');
    await writeFile(hook, failureHook);
    await writeFile(profile, JSON.stringify({
      remote: `http://127.0.0.1:${remote.address().port}`,
      serverToken: { kind: 'file', path: serverToken },
      localToken: { kind: 'file', path: localToken }, port: 0,
    }));
    child = spawn(process.execPath, ['--import', pathToFileURL(hook).href,
      launcher, 'serve', '--config', profile, ...(json ? ['--json'] : [])], {
      cwd: root, stdio: ['ignore', 'pipe', 'pipe'],
      env: { HOME: root, USERPROFILE: root, XDG_CONFIG_HOME: root,
        TMP: root, TEMP: root, TMPDIR: root, REMOTE_TEST_PHASE: phase,
        NODE_DISABLE_COMPILE_CACHE: '1',
        ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) },
    });
    let stdout = ''; let stderr = '';
    child.stdout.on('data', chunk => { stdout += chunk; });
    child.stderr.on('data', chunk => { stderr += chunk; });
    const [code, signal] = await once(child, 'close', { signal: context.signal });
    assert.equal(signal, null);
    assert.equal(code, 1, stderr);
    assert.doesNotMatch(stdout, /Stopped\.|"stopped":true/);
    if (phase === 'startup') assert.equal(stdout, '');
    else assert.match(stdout, /Local API:|"listening"/);
    assert.doesNotMatch(stderr, /private-close-canary|credential references/);
    if (json) {
      const result = JSON.parse(stderr);
      assert.equal(result.error, 'cleanup_failed');
      assert.equal(result.message, 'Remote cleanup failed.');
      assert.match(result.next, /same profile/);
    } else {
      assert.match(stderr, /^Remote cleanup failed\.\nNext: /);
      assert.match(stderr, /same profile/);
    }
  });
}
