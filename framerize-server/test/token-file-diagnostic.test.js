import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmodSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const launcher = fileURLToPath(new URL('../bin/framerize-server.js', import.meta.url));
for (const failure of ['missing', 'contents', 'permissions']) {
  test(`launcher explains and recovers from ${failure} in the server token file`, {
    skip: failure === 'permissions' && process.platform === 'win32',
  }, context => {
    const root = mkdtempSync(join(tmpdir(), 'server-token-diagnostic-'));
    context.after(() => rmSync(root, { recursive: true, force: true }));
    const tokenFile = join(root, 'private-path-canary');
    const token = 'synthetic-token-canary';
    if (failure !== 'missing') writeFileSync(tokenFile,
      failure === 'contents' ? `${token} invalid` : token, { mode: 0o600 });
    if (failure === 'permissions') chmodSync(tokenFile, 0o644);
    const invoke = command => spawnSync(process.execPath,
      [launcher, command, '--server-token-file', tokenFile], {
        cwd: root, encoding: 'utf8', timeout: 5000,
        env: { HOME: root, USERPROFILE: root, TMPDIR: root, NODE_DISABLE_COMPILE_CACHE: '1' },
      });
    for (const command of ['check', 'serve']) {
      const result = invoke(command);
      assert.equal(result.error, undefined);
      assert.equal(result.status, 1);
      assert.equal(result.stdout, '');
      const diagnostic = JSON.parse(result.stderr);
      assert.equal(diagnostic.event, 'server.failed');
      assert.equal(diagnostic.message, 'Server token file is unavailable or invalid.');
      assert.match(diagnostic.next, /--server-token-file/);
      assert.match(diagnostic.next, /contents/);
      assert.match(diagnostic.next, /Unix.*chmod 600/);
      assert.doesNotMatch(result.stderr, /private-path-canary|synthetic-token-canary/);
    }
    writeFileSync(tokenFile, token);
    chmodSync(tokenFile, 0o600);
    const recovered = invoke('check');
    assert.equal(recovered.error, undefined);
    assert.equal(recovered.status, 0, recovered.stderr);
    assert.doesNotMatch(recovered.stdout + recovered.stderr, /private-path-canary|synthetic-token-canary/);
  });
}
