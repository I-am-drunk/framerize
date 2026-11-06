import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import { getEventListeners, once } from 'node:events';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import { build } from 'esbuild';

const sourceRoot = process.env.FRAMERIZE_SOURCE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
const compiled = await build({ absWorkingDir: sourceRoot, stdin: {
  contents: "export { readSettings } from './framerize-server/src/settings.ts';"
    + "export { loadConfigFile } from './framerize-remote/src/configStore.ts';",
  resolveDir: sourceRoot,
}, bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
const inputs = await Promise.all(Object.keys(compiled.metafile.inputs)
  .filter(path => path !== '<stdin>').sort().map(async path => ({
    path, sha256: digest(await fs.readFile(resolve(sourceRoot, path))),
  })));
const bytes = compiled.outputFiles[0].contents;
console.log(JSON.stringify({ node: process.version, inputs, bundleSha256: digest(bytes) }));
const { readSettings, loadConfigFile } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

const readers = [{
  name: 'Server settings', load: readSettings,
  document: { version: 1, config: {}, environment: {} },
  invalid: error => error.message === 'Server settings are unavailable or invalid',
}, {
  name: 'Remote profile', load: loadConfigFile,
  document: { remote: 'http://127.0.0.1:8789', port: 0 },
  invalid: error => error.code === 'profile_invalid',
}];

for (const reader of readers) {
  test(`${reader.name} preserves cancellation when invalid bytes arrive`, { timeout: 3000 }, async context => {
    const root = await fs.mkdtemp(join(tmpdir(), 'saved-config-abort-'));
    const path = join(root, 'config.json');
    const content = Buffer.from([0xff]);
    const controller = new AbortController();
    const reason = new Error('synthetic read cancellation');
    const originalOpen = fs.open;
    let handle, closed;
    context.after(async () => {
      fs.open = originalOpen;
      syncBuiltinESMExports();
      await handle?.close();
      await fs.rm(root, { recursive: true, force: true });
    });
    await fs.writeFile(path, content, { mode: 0o600 });
    fs.open = async (target, ...options) => {
      const file = await originalOpen(target, ...options);
      if (target !== path) return file;
      handle = file;
      closed = once(file, 'close');
      const read = file.readFile.bind(file);
      file.readFile = async options => {
        const result = await read(options);
        controller.abort(reason);
        return result;
      };
      return file;
    };
    syncBuiltinESMExports();
    await assert.rejects(reader.load(path, controller.signal), error => error === reason);
    assert.ok(handle, 'the real file must have opened before cancellation');
    await closed;
    assert.equal(getEventListeners(controller.signal, 'abort').length, 0);
    assert.deepEqual(await fs.readFile(path), content);
    context.diagnostic('Original cancellation preserved; file closed and listeners removed');
  });
}

for (const reader of readers) {
  test(`${reader.name} keeps a leading UTF-8 BOM invalid`, { timeout: 3000 }, async context => {
    const root = await fs.mkdtemp(join(tmpdir(), 'saved-config-bom-'));
    context.after(() => fs.rm(root, { recursive: true, force: true }));
    const path = join(root, 'config.json');
    const content = Buffer.from('\uFEFF' + JSON.stringify(reader.document));
    await fs.writeFile(path, content, { mode: 0o600 });
    await assert.rejects(reader.load(path), reader.invalid);
    assert.deepEqual(await fs.readFile(path), content);
  });
}
