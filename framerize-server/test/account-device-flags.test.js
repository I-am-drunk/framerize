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
  contents: "export { loadAccounts, readAccountSession } from './accounts.ts';",
  resolveDir: fileURLToPath(new URL('../src/', import.meta.url)), loader: 'ts',
}, bundle: true, platform: 'node', format: 'esm', write: false });
const { loadAccounts, readAccountSession } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const cases = [
  { name: 'metadata', read: path => loadAccounts(path), text: '[]\n', expected: [],
    message: 'Account metadata unavailable' },
  { name: 'session', read: path => readAccountSession({ file: path }, {}),
    text: 'synthetic-session\n', expected: 'synthetic-session', message: 'Account session unavailable' },
];

for (const entry of cases) {
  test(`${entry.name} device rejection cannot acquire a controlling terminal`,
    { skip: process.platform === 'win32', timeout: 3000 }, async context => {
      const directory = await fs.mkdtemp(join(tmpdir(), 'account-device-'));
      context.after(async () => {
        context.mock.restoreAll();
        syncBuiltinESMExports();
        await fs.rm(directory, { recursive: true, force: true });
      });
      const path = join(directory, 'device');
      const calls = [];
      let reads = 0, closes = 0;
      context.mock.method(fs, 'open', async (requested, flags) => {
        calls.push({ requested, flags });
        return { stat: async () => ({ isFile: () => false, size: 0, mode: 0o600 }),
          read: async () => { reads++; throw new Error('Device must not be read'); },
          close: async () => { closes++; } };
      });
      syncBuiltinESMExports();
      await assert.rejects(entry.read(path), { message: entry.message });
      assert.equal(closes, 1);
      assert.equal(reads, 0);
      assert.equal(calls.length, 1);
      assert.equal(calls[0].requested, path);
      assert.equal(calls[0].flags & constants.O_NONBLOCK, constants.O_NONBLOCK);
      assert.equal(calls[0].flags & (constants.O_WRONLY | constants.O_RDWR), 0);
      assert.equal(calls[0].flags & constants.O_NOCTTY, constants.O_NOCTTY,
        'Rejected account devices must not become the controlling terminal');
    });

  test(`${entry.name} device guard preserves regular-file reads`, { timeout: 3000 }, async context => {
    const directory = await fs.mkdtemp(join(tmpdir(), 'account-device-control-'));
    context.after(() => fs.rm(directory, { recursive: true, force: true }));
    const path = join(directory, 'regular');
    await fs.writeFile(path, entry.text, { mode: 0o600 });
    assert.deepEqual(await entry.read(path), entry.expected);
    assert.equal(await fs.readFile(path, 'utf8'), entry.text);
  });
}
