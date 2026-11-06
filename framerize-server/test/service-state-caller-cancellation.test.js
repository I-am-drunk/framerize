import assert from 'node:assert/strict';
import test from 'node:test';
import fs from 'node:fs/promises';
import cp from 'node:child_process';
import net from 'node:net';
import { getEventListeners } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { runSetupCommand } from './setup.ts'; export { selectServiceManager } from './service-units.ts';",
  resolveDir: fileURLToPath(new URL('../src/', import.meta.url)), loader: 'ts',
}, bundle: true, platform: 'node', format: 'esm', write: false,
plugins: [{ name: 'source-boundaries', setup(builder) {
  builder.onResolve({ filter: /^\.\/(backend|account-cli|browser-login)\.js$/ },
    args => ({ path: args.path, external: true }));
  builder.onResolve({ filter: /\/dist\/core\.js$/ }, () => ({
    path: fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)),
  }));
} }], });
stop();
const { runSetupCommand, selectServiceManager } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function within(pending, phase) {
  let timer;
  try { return await Promise.race([pending, new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`Timed out during ${phase}`)), 1500);
  })]); } finally { clearTimeout(timer); }
}

const cases = [[['status'], 1], [['service', 'status'], 1],
  [['service', 'start'], 1], [['service', 'stop'], 1], [['service', 'restart'], 1],
  [['service', 'uninstall'], 1], [['service', 'uninstall'], 2]];
for (const [argv, occurrence] of cases) {
  test(`${argv.join(' ')} cancels unit read ${occurrence} before further work`,
    { timeout: 6000 }, async context => {
      const root = await fs.mkdtemp(join(tmpdir(), 'server-state-caller-'));
      const controller = new AbortController(), reason = new Error('Synthetic cancellation');
      const entered = Promise.withResolvers(), gate = Promise.withResolvers();
      const closed = Promise.withResolvers(), handles = [], reads = [], commands = [];
      let pending, target, count = 0, guards = 0, output = '', cleaned = false;
      const cleanup = async () => {
        if (cleaned) return;
        controller.abort(reason);
        gate.resolve();
        if (pending) await within(pending, 'caller cleanup settlement');
        await within(Promise.allSettled(reads), 'caller read cleanup');
        for (const handle of handles) if (handle.fd >= 0) await handle.close();
        context.mock.restoreAll();
        syncBuiltinESMExports();
        await fs.rm(root, { recursive: true, force: true });
        cleaned = true;
      };
      context.after(cleanup);
      const home = join(root, 'home'), xdg = join(root, 'config');
      const program = [process.execPath, join(root, 'unused.js')];
      const plan = { home, homeDir: root, xdgConfigHome: xdg, platform: 'linux', uid: 7342,
        program: [...program, 'serve', '--config', join(home, 'server.json')],
        environmentPath: [dirname(program[0]), '/opt/homebrew/bin', '/usr/local/bin', '/usr/bin', '/bin'].join(':') };
      const manager = selectServiceManager(plan);
      await fs.mkdir(dirname(manager.path), { recursive: true });
      await fs.writeFile(manager.path, manager.contents, { mode: 0o600 });
      const originalOpen = fs.open.bind(fs);
      context.mock.method(fs, 'open', async (path, ...args) => {
        assert.equal(path, manager.path);
        const handle = await originalOpen(path, ...args);
        handles.push(handle);
        const read = handle.readFile.bind(handle), close = handle.close.bind(handle);
        context.mock.method(handle, 'readFile', options => {
          assert.equal(options.signal, controller.signal);
          const ordinal = ++count;
          const operation = read(options).then(async bytes => {
            if (ordinal === occurrence) {
              target = handle;
              entered.resolve();
              await gate.promise;
            }
            return bytes;
          });
          reads.push(operation);
          return operation;
        });
        context.mock.method(handle, 'close', async () => {
          await close();
          if (handle === target) closed.resolve();
        });
        return handle;
      });
      const forbidden = () => { guards++; throw new Error('Unexpected process or network call'); };
      context.mock.method(cp, 'spawn', forbidden);
      context.mock.method(cp, 'execFile', forbidden);
      context.mock.method(net, 'createServer', forbidden);
      context.mock.method(globalThis, 'fetch', forbidden);
      syncBuiltinESMExports();
      pending = runSetupCommand([...argv, '--json'], {
        signal: controller.signal, platform: 'linux', uid: 7342, program,
        env: { HOME: root, FRAMERIZE_SERVER_HOME: home, XDG_CONFIG_HOME: xdg },
        stdout: { write(text) { output += text; } },
        ready: forbidden,
        run: async (command, args) => {
          commands.push({ command, args: [...args], afterAbort: controller.signal.aborted });
          assert.equal(command, 'systemctl');
          assert.equal(controller.signal.aborted, false);
          assert.ok(['show', 'is-enabled'].includes(args[1]), 'No service mutation');
          return { code: 0, stdout: args[1] === 'show'
            ? 'ActiveState=inactive\nSubState=dead\nMainPID=0\n' : 'enabled\n' };
        },
      }).then(value => ({ value }), error => ({ error }));
      await within(entered.promise, 'caller read entry');
      controller.abort(reason);
      assert.equal((await within(pending, 'caller cancellation before release')).error, reason);
      await within(closed.promise, 'caller close before release');
      assert.equal(target.fd, -1);
      gate.resolve();
      await within(Promise.allSettled(reads), 'caller late reads');
      assert.ok(handles.every(handle => handle.fd === -1));
      assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
      assert.equal(commands.length, occurrence === 1 ? 0 : 2);
      assert.equal(guards, 0);
      assert.equal(output, '');
      assert.equal(await fs.readFile(manager.path, 'utf8'), manager.contents);
      await assert.rejects(fs.access(join(home, 'server.json')), { code: 'ENOENT' });
      await cleanup();
      await assert.rejects(fs.access(root), { code: 'ENOENT' });
    });
}
