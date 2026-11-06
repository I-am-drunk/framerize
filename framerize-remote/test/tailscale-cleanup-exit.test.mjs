import test, { after } from 'node:test';
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const output = await build({ entryPoints: [fileURLToPath(new URL('../src/tailscale.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false });
stop();
const shared = await mkdtemp(join(tmpdir(), 'frm-tailscale-exit-bundle-'));
const bundle = join(shared, 'bundle.mjs');
await writeFile(bundle, output.outputFiles[0].contents);
after(() => rm(shared, { recursive: true, force: true }));
const helper = fileURLToPath(new URL('./helpers/tailscale-cleanup-exit-caller.mjs', import.meta.url));

async function within(pending, milliseconds) {
  let timer;
  try { return await Promise.race([pending, new Promise(resolve => {
    timer = setTimeout(() => resolve(null), milliseconds);
  })]); } finally { clearTimeout(timer); }
}

for (const [platform, mode] of [['linux', 'deny'], ['win32', 'deny'], ['linux', 'allow']]) {
  test(`caller exits after ${platform} ${mode} cleanup`, {
    timeout: 7000, skip: process.platform === 'win32' && 'Uses POSIX synthetic children',
  }, async context => {
    const root = await mkdtemp(join(tmpdir(), 'frm-tailscale-exit-'));
    const receipt = join(root, 'child.json');
    let caller, closed;
    context.after(async () => {
      try {
        const record = JSON.parse(await readFile(receipt, 'utf8'));
        assert.ok(Number.isSafeInteger(record.pid) && record.pid > 1);
        try { process.kill(record.group ? -record.pid : record.pid, 'SIGKILL'); }
        catch (error) { if (error.code !== 'ESRCH') throw error; }
      } catch (error) { if (error.code !== 'ENOENT') throw error; }
      finally {
        if (caller?.exitCode === null && caller.signalCode === null) caller.kill('SIGKILL');
        if (closed) assert.notEqual(await within(closed, 1500), null, 'Caller must close during cleanup');
        await rm(root, { recursive: true, force: true });
      }
    });
    caller = spawn(process.execPath, [helper, bundle, receipt, platform, mode], {
      cwd: root, env: { HOME: root, TMPDIR: root, PATH: '' },
      stdio: ['ignore', 'pipe', 'pipe', 'ipc'],
    });
    let stderr = '';
    caller.stdout.resume();
    caller.stderr.on('data', chunk => { stderr += chunk; });
    const reported = new Promise((resolve, reject) => {
      caller.once('message', resolve);
      caller.once('error', reject);
    });
    closed = new Promise(resolve => caller.once('close', (code, signal) => resolve({ code, signal })));
    const result = await within(reported, 3000);
    assert.deepEqual(result, mode === 'deny' ? { error: 'service_cleanup_failed' } : { state: 'cancelled' }, stderr);
    const outcome = await within(closed, 1000);
    const record = JSON.parse(await readFile(receipt, 'utf8'));
    let childAlive;
    try { process.kill(record.pid, 0); childAlive = true; }
    catch (error) {
      if (error.code !== 'ESRCH') throw error;
      childAlive = false;
    }
    context.diagnostic(JSON.stringify({ platform, mode, result, outcome, childAlive }));
    assert.equal(childAlive, mode === 'deny');
    assert.deepEqual(outcome, { code: mode === 'deny' ? 1 : 0, signal: null },
      'The caller must exit after reporting the cleanup outcome');
  });
}
