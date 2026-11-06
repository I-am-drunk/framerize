import assert from 'node:assert/strict';
import test from 'node:test';
import { constants } from 'node:fs';
import fs from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { readText } from './versions.ts';",
  resolveDir: fileURLToPath(new URL('../src/compatibility/', import.meta.url)),
  loader: 'ts',
}, bundle: true, platform: 'node', format: 'esm', write: false });
const { readText } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

test('settings device rejection does not allow acquiring a terminal',
  { skip: process.platform === 'win32', timeout: 3000 }, async context => {
    const directory = await fs.mkdtemp(join(tmpdir(), 'frm-settings-device-'));
    context.after(async () => {
      context.mock.restoreAll();
      syncBuiltinESMExports();
      await fs.rm(directory, { recursive: true, force: true });
    });
    const path = join(directory, 'synthetic-device');
    const opened = [];
    let closes = 0, reads = 0;
    context.mock.method(fs, 'open', async (requested, flags) => {
      opened.push({ requested, flags });
      return { stat: async () => ({ isFile: () => false }),
        read: async () => { reads++; throw new Error('Device must not be read'); },
        close: async () => { closes++; } };
    });
    syncBuiltinESMExports();
    assert.equal(await readText(path), null);
    assert.equal(closes, 1);
    assert.equal(reads, 0);
    assert.equal(opened.length, 1);
    assert.equal(opened[0].requested, path);
    assert.equal(opened[0].flags & constants.O_NONBLOCK, constants.O_NONBLOCK);
    assert.equal(opened[0].flags & (constants.O_WRONLY | constants.O_RDWR), 0);
    assert.equal(opened[0].flags & constants.O_NOCTTY, constants.O_NOCTTY,
      'Opening rejected settings devices must not acquire a controlling terminal');
  });

test('settings device guard preserves regular-file contents',
  { timeout: 3000 }, async context => {
    const directory = await fs.mkdtemp(join(tmpdir(), 'frm-settings-device-control-'));
    context.after(() => fs.rm(directory, { recursive: true, force: true }));
    const path = join(directory, 'settings.json');
    const original = '{"autoUpdatesChannel":"stable","label":"synthetic 日本語"}\n';
    await fs.writeFile(path, original, { mode: 0o600 });
    assert.equal(await readText(path), original);
    assert.equal(await fs.readFile(path, 'utf8'), original);
    await fs.rename(path, `${path}.moved`);
    await fs.rm(`${path}.moved`);
  });
