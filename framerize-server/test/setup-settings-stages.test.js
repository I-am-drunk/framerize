import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const core = fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url));
const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/setup.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false,
  plugins: [{ name: 'source-only', setup(builder) {
    builder.onResolve({ filter: /^\.\/(backend|account-cli|browser-login)\.js$/ }, args =>
      ({ path: args.path, external: true }));
    builder.onResolve({ filter: /\/dist\/core\.js$/ }, () => ({ path: core }));
  } }],
});
stop();
const { runSetupCommand } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function within(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((resolveUnused, reject) => {
      timer = setTimeout(() => reject(new Error('Setup did not settle')), 1000);
    })]);
  } finally { clearTimeout(timer); }
}

const cases = [
  { name: 'up effective settings', args: ['up', '--no-lan', '--json'], read: 2 },
  { name: 'pair effective settings', args: ['pair', '--json'], read: 2 },
  { name: 'uninstall saved settings', args: ['service', 'uninstall', '--json'], read: 1 },
];

for (const entry of cases) {
  test(`cancellation stops ${entry.name}`, { timeout: 5000 }, async context => {
    const root = await fs.mkdtemp(join(tmpdir(), 'setup-settings-stage-'));
    const path = join(root, 'server.json');
    const originalRead = fs.readFile;
    const originalOpen = fs.open;
    const entered = Promise.withResolvers();
    const release = Promise.withResolvers();
    const controller = new AbortController();
    const reason = new Error('synthetic stage cancellation');
    const commands = [];
    const expectedCommands = entry.args[0] === 'service'
      ? [['systemctl', '--user', 'is-enabled', 'framerize-server.service']] : [];
    let pending, output = '', reads = 0, ready = 0;
    context.after(async () => {
      release.resolve();
      try { if (pending) await within(pending); }
      finally {
        context.mock.restoreAll();
        syncBuiltinESMExports();
        await fs.rm(root, { recursive: true, force: true });
        await assert.rejects(fs.stat(root), { code: 'ENOENT' });
      }
    });
    const settings = JSON.stringify({ version: 1, config: {
      backend: 'http', upstream: 'http://127.0.0.1:9', host: '127.0.0.1',
      port: 8789, privateNetworkOnly: false }, environment: {} });
    await fs.writeFile(path, settings, { mode: 0o600 });
    context.mock.method(fs, 'open', async function (file, ...options) {
      const handle = await originalOpen.call(this, file, ...options);
      if (String(file) === path) {
        const read = handle.readFile.bind(handle);
        context.mock.method(handle, 'readFile', async (...args) => {
          const value = await read(...args);
          if (++reads === entry.read) {
            entered.resolve({ entered: true });
            await release.promise;
          }
          return value;
        });
      }
      return handle;
    });
    syncBuiltinESMExports();
    pending = runSetupCommand(entry.args, {
      signal: controller.signal, platform: 'linux', uid: 501,
      env: { HOME: root, XDG_CONFIG_HOME: join(root, 'config'), FRAMERIZE_SERVER_HOME: root },
      program: [process.execPath, join(root, 'synthetic-server.js')],
      stdout: { write(text) { output += text; } },
      run: async (command, args) => {
        commands.push([command, ...args]);
        assert.deepEqual(commands, expectedCommands);
        return { code: 1, stdout: '' };
      },
      ready: async () => { ready++; throw new Error('Unexpected readiness'); },
    }).then(value => ({ value }), error => ({ error }));
    assert.equal((await within(Promise.race([entered.promise, pending]))).entered, true);
    controller.abort(reason);
    assert.equal((await within(pending)).error, reason);
    release.resolve();
    assert.equal(await originalRead(path, 'utf8'), settings);
    assert.deepEqual(commands, expectedCommands);
    assert.equal(ready, 0);
    assert.equal(output, '');
    assert.deepEqual(await fs.readdir(root), ['server.json']);
  });
}
