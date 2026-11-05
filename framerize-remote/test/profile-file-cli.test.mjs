import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { constants } from 'node:fs';
import { mkdtemp, open, readdir, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const packageRoot = process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
const launcher = join(packageRoot, 'bin/frm.js');

async function noFifoReader(path) {
  let handle;
  try {
    handle = await open(path, constants.O_WRONLY | constants.O_NONBLOCK);
    assert.fail('the CLI left a FIFO reader open');
  } catch (error) { assert.equal(error.code, 'ENXIO'); }
  finally { await handle?.close(); }
}

for (const command of ['check', 'serve']) for (const kind of ['fifo', 'missing', 'malformed']) {
  test(`frm ${command} diagnoses a ${kind} profile and exits`, {
    timeout: 8000, skip: kind === 'fifo' && process.platform === 'win32',
  }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'frm-profile-cli-'));
    const profile = join(root, 'framerize-remote.json');
    let fifoCreated = false;
    context.after(async () => {
      try { if (fifoCreated) await noFifoReader(profile); }
      finally { await rm(root, { recursive: true, force: true }); }
    });
    await writeFile(join(root, 'canary.txt'), 'preserve synthetic state', { mode: 0o600 });
    if (kind === 'fifo') {
      execFileSync('mkfifo', ['-m', '600', profile], { timeout: 2000 });
      fifoCreated = true;
    }
    if (kind === 'malformed') await writeFile(profile, '{', { mode: 0o600 });
    const before = (await readdir(root)).sort();
    const result = spawnSync(process.execPath, [launcher, command, '--config', profile, '--json'], {
      cwd: root, encoding: 'utf8', timeout: 3000, killSignal: 'SIGKILL', maxBuffer: 64 * 1024,
      env: { HOME: root, USERPROFILE: root, TMPDIR: root, TMP: root, TEMP: root,
        ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}) },
    });
    assert.equal(result.error, undefined, 'the CLI must finish before its deadline');
    assert.equal(result.signal, null);
    assert.equal(result.status, 1);
    assert.equal(result.stdout, '');
    const expected = kind === 'missing' ? 'profile_missing' : 'profile_invalid';
    const diagnostic = JSON.parse(result.stderr);
    assert.equal(diagnostic.error, expected);
    assert.equal(diagnostic.code, expected);
    assert.equal(typeof diagnostic.next, 'string');
    assert.ok(diagnostic.next.length > 0);
    assert.deepEqual((await readdir(root)).sort(), before);
    assert.equal(await readFile(join(root, 'canary.txt'), 'utf8'), 'preserve synthetic state');
    if (kind === 'malformed') assert.equal(await readFile(profile, 'utf8'), '{');
  });
}
