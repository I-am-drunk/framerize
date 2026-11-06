import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build, stop } from 'esbuild';

const root = fileURLToPath(new URL('../../', import.meta.url));
const compiled = await build({ absWorkingDir: root,
  entryPoints: ['framerize-remote/src/serviceControl.ts'],
  alias: { '@framerize/core': join(root, 'framerize-core/src/index.ts') },
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
stop();
const digest = value => createHash('sha256').update(value).digest('hex');
const api = await import(`data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
console.log(JSON.stringify({ runtime: process.version,
  inputs: Object.fromEntries(await Promise.all(Object.keys(compiled.metafile.inputs)
    .map(async path => [path, digest(await fs.readFile(join(root, path)))]))) }));

async function bounded(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_resolve, reject) => {
      timer = setTimeout(() => reject(new Error('Unit caller fixture timed out')), 1500);
    })]);
  } finally { clearTimeout(timer); }
}

for (const name of ['installService', 'snapshotService']) {
  test(`${name} cancels its pending unit read before any later service step`,
    { timeout: 5000 }, async context => {
      const home = await fs.mkdtemp(join(tmpdir(), 'frm-unit-read-caller-'));
      const path = join(home, '.config/systemd/user/frm.service');
      const unit = `Environment=FRM_HOME=${home}\n`;
      const controller = new AbortController();
      const entered = Promise.withResolvers(), release = Promise.withResolvers();
      const calls = [];
      let pending;
      context.after(async () => {
        controller.abort();
        release.resolve();
        try { if (pending) await bounded(pending); }
        finally {
          context.mock.restoreAll();
          syncBuiltinESMExports();
          await fs.rm(home, { recursive: true, force: true });
          await assert.rejects(fs.access(home), { code: 'ENOENT' });
        }
      });
      await fs.mkdir(dirname(path), { recursive: true });
      await fs.writeFile(path, unit, { mode: 0o600 });
      await fs.writeFile(join(home, 'framerize-remote.json'),
        JSON.stringify({ remote: 'http://127.0.0.1:9999', port: 8788 }), { mode: 0o600 });
      const original = fs.readFile;
      context.mock.method(fs, 'readFile', async (...args) => {
        const value = await original(...args);
        if (String(args[0]) === path) {
          assert.equal(args[1].signal, controller.signal);
          entered.resolve({ entered: true });
          await release.promise;
        }
        return value;
      });
      syncBuiltinESMExports();
      pending = api[name]({ host: { platform: 'linux', homeDir: home, uid: 1000 },
        home, program: ['/synthetic/frm', 'serve'], environmentPath: '/synthetic/bin',
        signal: controller.signal, run: async (command, args) => {
          calls.push({ command, args });
          return { code: 0, stdout: 'yes\n' };
        } }).then(value => ({ value }), error => ({ error }));
      assert.equal((await bounded(Promise.race([entered.promise, pending]))).entered, true);
      const callsAtRead = calls.length;
      controller.abort(new Error('Synthetic unit caller cancellation'));
      const result = await bounded(pending);
      assert.equal(result.error?.code, 'aborted');
      assert.equal(calls.length, callsAtRead);
      assert.equal(await original(path, 'utf8'), unit);
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
      release.resolve();
    });
}
