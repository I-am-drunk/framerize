import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build, stop } from 'esbuild';

const core = fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url));
const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/setup.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
  plugins: [{ name: 'source-core', setup(builder) {
    builder.onResolve({ filter: /^\.\/(backend|account-cli|browser-login)\.js$/ }, args =>
      ({ path: args.path, external: true }));
    builder.onResolve({ filter: /\/dist\/core\.js$/ }, () => ({ path: core }));
  } }],
});
stop();
assert.ok(Object.keys(compiled.metafile.inputs).every(path => !/castle/i.test(path)));
const bytes = compiled.outputFiles[0].contents;
const bundleHash = createHash('sha256').update(bytes).digest('hex');
const { runSetupCommand } = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

async function bounded(promise, milliseconds = 1000) {
  let timer;
  try {
    return await Promise.race([promise, new Promise(resolve => {
      timer = setTimeout(() => resolve({ timedOut: true }), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

test('setup cancels a pending settings read before its completion', { timeout: 5000 }, async context => {
  const root = await fs.mkdtemp(join(tmpdir(), 'server-settings-cancel-'));
  const path = join(root, 'server.json');
  const originalRead = fs.readFile;
  const originalOpen = fs.open;
  const entered = Promise.withResolvers();
  const released = Promise.withResolvers();
  const controller = new AbortController();
  const reason = new Error('synthetic settings-read cancellation');
  const commands = [];
  let pending;
  let held = false;
  let output = '';
  context.after(async () => {
    released.resolve();
    const settled = pending ? await bounded(pending) : {};
    context.mock.restoreAll();
    syncBuiltinESMExports();
    await fs.rm(root, { recursive: true, force: true });
    await assert.rejects(fs.stat(root), { code: 'ENOENT' });
    assert.notEqual(settled.timedOut, true);
  });
  const settings = Buffer.from(JSON.stringify({ version: 1,
    config: { backend: 'http', upstream: 'http://127.0.0.1:9', host: '127.0.0.1',
      port: 8789, privateNetworkOnly: false }, environment: {} }));
  await fs.writeFile(path, settings, { mode: 0o600 });
  context.mock.method(fs, 'open', async function (file, ...args) {
    const handle = await originalOpen.call(this, file, ...args);
    if (String(file) === path) {
      const read = handle.readFile.bind(handle);
      context.mock.method(handle, 'readFile', async (...options) => {
        const result = await read(...options);
        if (!held) {
          held = true;
          entered.resolve({ entered: true });
          await released.promise;
        }
        return result;
      });
    }
    return handle;
  });
  syncBuiltinESMExports();
  pending = runSetupCommand(['up', '--no-lan', '--json'], {
    signal: controller.signal, platform: 'linux', uid: 501,
    env: { HOME: root, XDG_CONFIG_HOME: join(root, 'config'), FRAMERIZE_SERVER_HOME: root },
    program: [process.execPath, join(root, 'synthetic-server.js')],
    stdout: { write(text) { output += text; } },
    run: async (command, args) => { commands.push([command, ...args]); throw new Error('Unexpected service command'); },
    ready: async () => { throw new Error('Unexpected readiness request'); },
  }).then(value => ({ value }), error => ({ error }));
  const started = await bounded(Promise.race([entered.promise, pending]));
  assert.equal(started.entered, true, 'Setup must reach the private settings read');
  controller.abort(reason);
  const beforeRelease = await bounded(pending);
  const beforeCommands = [...commands];
  const beforeOutput = output;
  released.resolve();
  const afterRelease = await bounded(pending);
  const afterBytes = await originalRead(path);
  const remaining = await fs.readdir(root);
  context.diagnostic(JSON.stringify({ synthetic: true, bundleHash,
    settledBeforeRelease: beforeRelease.timedOut !== true,
    cancellationReasonAfterRelease: afterRelease.error === reason,
    commandsBeforeRelease: beforeCommands, commandsAfterRelease: commands,
    outputBeforeRelease: beforeOutput, outputAfterRelease: output,
    fileUnchanged: afterBytes.equals(settings), remaining }));
  assert.equal(afterRelease.error, reason, 'Late read completion must preserve cancellation');
  assert.deepEqual(commands, []);
  assert.equal(output, '');
  assert.deepEqual(afterBytes, settings);
  assert.deepEqual(remaining, ['server.json']);
  assert.equal(beforeRelease.error, reason, 'Setup must cancel without waiting for the settings read');
});
