import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build, stop } from 'esbuild';

const compiled = await build({
  entryPoints: [fileURLToPath(new URL('../src/setup.ts', import.meta.url))],
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true,
  external: ['./backend.js'], plugins: [{ name: 'source-core', setup(builder) {
    builder.onResolve({ filter: /\/dist\/core\.js$/ }, () => ({
      path: fileURLToPath(new URL('../../framerize-core/src/index.ts', import.meta.url)),
    }));
  } }],
});
stop();
assert.ok(Object.keys(compiled.metafile.inputs).every(path => !/castle/i.test(path)));
const bundle = compiled.outputFiles[0].contents;
const bundleHash = createHash('sha256').update(bundle).digest('hex');
const { runSetupCommand } = await import(`data:text/javascript;base64,${Buffer.from(bundle).toString('base64')}`);

async function bounded(pending, milliseconds = 1500) {
  let timer;
  try { return await Promise.race([pending, new Promise((resolveUnused, reject) => {
    timer = setTimeout(() => reject(new Error('Owned fixture wait expired')), milliseconds);
  })]); } finally { clearTimeout(timer); }
}

for (const phase of ['open', 'read']) test(`setup cancels a pending server-token ${phase}`,
  { timeout: 5000 }, async context => {
    const root = await fs.mkdtemp(join(tmpdir(), 'server-token-cancel-'));
    const tokenPath = join(root, 'synthetic-token');
    const home = join(root, 'server-home');
    const xdg = join(root, 'xdg');
    const token = 'synthetic-cancellation-token\n';
    const controller = new AbortController();
    const reason = new Error('Synthetic setup cancellation');
    const originalOpen = fs.open;
    let release, notifyPending, notifyClosed, pending, handle, mock;
    let blocked = false;
    let closes = 0;
    let output = '';
    const commands = [];
    const held = new Promise(resolve => { release = resolve; });
    const started = new Promise(resolve => { notifyPending = resolve; });
    const closed = new Promise(resolve => { notifyClosed = resolve; });
    context.after(async () => {
      controller.abort(reason);
      release();
      try { if (pending) await bounded(pending); }
      finally {
        if (handle && handle.fd !== -1) await handle.close();
        mock?.mock.restore();
        syncBuiltinESMExports();
        await fs.rm(root, { recursive: true, force: true });
        await assert.rejects(fs.stat(root), { code: 'ENOENT' });
        context.diagnostic('Held work released, captured descriptor closed, private state removed.');
      }
    });
    await fs.writeFile(tokenPath, token, { mode: 0o600 });
    const hold = async () => { blocked = true; notifyPending(); await held; };
    mock = context.mock.method(fs, 'open', async (path, ...args) => {
      if (path !== tokenPath) return originalOpen(path, ...args);
      if (phase === 'open') await hold();
      handle = await originalOpen(path, ...args);
      const originalClose = handle.close.bind(handle);
      context.mock.method(handle, 'close', async () => {
        await originalClose();
        closes++;
        notifyClosed();
      });
      if (phase === 'read') {
        const originalRead = handle.read.bind(handle);
        context.mock.method(handle, 'read', async (...readArgs) => {
          if (!blocked) await hold();
          return originalRead(...readArgs);
        });
      }
      return handle;
    });
    syncBuiltinESMExports();
    pending = runSetupCommand(['up', '--backend', 'http', '--upstream', 'http://127.0.0.1:1',
      '--host', '127.0.0.1', '--port', '8789', '--no-lan', '--server-token-file', tokenPath, '--json'], {
      env: { HOME: root, USERPROFILE: root, FRAMERIZE_SERVER_HOME: home, XDG_CONFIG_HOME: xdg },
      stdout: { write(text) { output += text; } }, signal: controller.signal,
      platform: 'linux', uid: 501, program: ['/synthetic/node', '/synthetic/server.js'],
      run: async (command, args) => {
        commands.push([command, ...args]);
        throw new Error('Cancelled token read must not invoke service commands');
      },
    }).then(value => ({ value }), error => ({ error }));
    await bounded(started);
    controller.abort(reason);
    const settledBeforeRelease = await bounded(pending.then(() => true), 400).catch(() => false);
    release();
    const outcome = await bounded(pending);
    await bounded(closed);
    assert.equal(handle.fd, -1, 'The captured token descriptor must be closed');
    assert.equal(closes, 1, 'The captured token descriptor must close exactly once');
    assert.equal(await fs.readFile(tokenPath, 'utf8'), token);
    await assert.rejects(fs.stat(home), { code: 'ENOENT' });
    await assert.rejects(fs.stat(xdg), { code: 'ENOENT' });
    assert.equal(output, '');
    assert.deepEqual(commands, []);
    assert.equal(outcome.error, reason, 'Cancellation must preserve its original reason');
    context.diagnostic(JSON.stringify({ phase, bundleHash,
      inputs: Object.keys(compiled.metafile.inputs), settledBeforeRelease,
      preservedReason: outcome.error === reason, closes, descriptor: handle.fd,
      noSettingsOrUnitWritten: true, commandCount: commands.length, outputBytes: output.length,
    }));
    assert.equal(settledBeforeRelease, true, 'Setup cancellation must not await the held token operation');
  });
