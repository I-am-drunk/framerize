import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs/promises';
import { readFileSync } from 'node:fs';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build, stop } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { reconcileService } from './service-control.ts'; export { selectServiceManager } from './service-units.ts';",
  resolveDir: fileURLToPath(new URL('../src/', import.meta.url)), loader: 'ts',
}, bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
stop();
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
console.log(JSON.stringify({ synthetic: true, bundle: hash(compiled.outputFiles[0].contents),
  inputs: Object.keys(compiled.metafile.inputs).filter(path => path !== '<stdin>')
    .map(path => ({ path, sha256: hash(readFileSync(path)) })) }));
const { reconcileService, selectServiceManager } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function observe(pending, milliseconds) {
  let timer;
  try { return await Promise.race([pending, new Promise(resolve => {
    timer = setTimeout(() => resolve({ timedOut: true }), milliseconds);
  })]); } finally { clearTimeout(timer); }
}

for (const phase of ['status-unit', 'snapshot-settings', 'snapshot-unit']) {
  test(`Server cancellation interrupts ${phase} reads before service mutation`,
    { timeout: 5000 }, async context => {
      const root = await fs.mkdtemp(join(tmpdir(), 'server-snapshot-cancel-'));
      const controller = new AbortController();
      const reason = new Error('Synthetic setup cancellation');
      const commands = [], reads = [], handles = [];
      let release = () => {}, pending, cleaned = false, readyCalls = 0, preflights = 0;
      const cleanup = async () => {
        if (cleaned) return;
        controller.abort(reason);
        release();
        if (pending) assert.equal((await observe(pending, 1500)).timedOut, undefined);
        await Promise.allSettled(reads);
        for (const handle of handles) if (handle.fd >= 0) await handle.close();
        context.mock.restoreAll();
        syncBuiltinESMExports();
        await fs.rm(root, { recursive: true, force: true });
        cleaned = true;
      };
      context.after(cleanup);
      const plan = { platform: 'linux', uid: 7342, home: join(root, 'home'), homeDir: root,
        xdgConfigHome: join(root, 'config'), program: [process.execPath, join(root, 'unused.js')],
        environmentPath: '/synthetic/bin' };
      const manager = selectServiceManager(plan);
      const settingsPath = join(plan.home, 'server.json');
      const previous = '{"config":{"backend":"http","port":9432}}\n';
      await fs.mkdir(dirname(manager.path), { recursive: true, mode: 0o700 });
      await fs.mkdir(plan.home, { mode: 0o700 });
      await fs.writeFile(manager.path, manager.contents, { mode: 0o600 });
      await fs.writeFile(settingsPath, previous, { mode: 0o600 });
      const originalRead = fs.readFile.bind(fs), originalOpen = fs.open.bind(fs);
      const target = phase === 'snapshot-settings' ? settingsPath : manager.path;
      const occurrence = phase === 'snapshot-unit' ? 2 : 1;
      let count = 0, enteredResolve;
      const entered = new Promise(resolve => { enteredResolve = resolve; });
      const hold = (path, operation) => {
        const ordinal = String(path) === target ? ++count : 0;
        const result = Promise.resolve(operation).then(async value => {
          if (ordinal === occurrence) {
            await new Promise(resolve => { release = resolve; enteredResolve(true); });
          }
          return value;
        });
        reads.push(result);
        return result;
      };
      context.mock.method(fs, 'readFile', (path, ...args) => hold(path, originalRead(path, ...args)));
      context.mock.method(fs, 'open', async (path, ...args) => {
        const handle = await originalOpen(path, ...args);
        handles.push(handle);
        const read = handle.readFile.bind(handle);
        context.mock.method(handle, 'readFile', (...options) => hold(path, read(...options)));
        return handle;
      });
      syncBuiltinESMExports();
      const run = async (command, args) => {
        commands.push({ command, args: [...args], afterAbort: controller.signal.aborted });
        assert.equal(command, 'systemctl');
        if (args[1] === 'show') return { code: 0,
          stdout: 'ActiveState=inactive\nSubState=dead\nMainPID=0\n' };
        assert.equal(args[1], 'is-enabled', 'No service mutation may be invoked');
        return { code: 0, stdout: 'enabled\n' };
      };
      pending = reconcileService(plan, '{"config":{"backend":"http","port":9433}}\n',
        async () => { readyCalls++; return 0; }, run, async () => { preflights++; },
        controller.signal).then(value => ({ value }), error => ({ error }));
      assert.equal(await observe(entered, 1000), true, 'The selected real-file read must be held');
      controller.abort(reason);
      const beforeRelease = await observe(pending, 300);
      const unitBeforeRelease = await originalRead(manager.path, 'utf8');
      const settingsBeforeRelease = await originalRead(settingsPath, 'utf8');
      release();
      const afterRelease = await observe(pending, 1000);
      await Promise.allSettled(reads);
      const liveHandles = handles.filter(handle => handle.fd >= 0).length;
      const unitAfterRelease = await originalRead(manager.path, 'utf8');
      const settingsAfterRelease = await originalRead(settingsPath, 'utf8');
      const abortListeners = getEventListeners(controller.signal, 'abort').length;
      await cleanup();
      context.diagnostic(JSON.stringify({ phase, synthetic: true,
        rejectedBeforeRelease: beforeRelease.error === reason,
        rejectedAfterRelease: afterRelease.error === reason, readyCalls, preflights,
        commandsAfterAbort: commands.filter(entry => entry.afterAbort).length,
        liveHandles, abortListeners, privateStateRemoved: cleaned }));
      await assert.rejects(fs.access(root), { code: 'ENOENT' });
      assert.equal(unitBeforeRelease, manager.contents);
      assert.equal(unitAfterRelease, manager.contents);
      assert.equal(settingsBeforeRelease, previous);
      assert.equal(settingsAfterRelease, previous);
      assert.equal(readyCalls, 0);
      assert.equal(preflights, 0);
      assert.equal(liveHandles, 0);
      assert.equal(abortListeners, 0);
      assert.ok(commands.every(entry => !entry.afterAbort));
      assert.equal(afterRelease.error, reason);
      assert.equal(beforeRelease.error, reason,
        'Cancellation must reject before the held service-state read is released');
    });
}
