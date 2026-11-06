import assert from 'node:assert/strict';
import test from 'node:test';
import { spawnSync } from 'node:child_process';
import { constants } from 'node:fs';
import { mkdtemp, open, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const source = fileURLToPath(new URL('../src/compatibility/', import.meta.url));
const compiled = await build({ stdin: {
  contents: "export { lookupLatestVersion } from './latest.ts'; export { readText } from './versions.ts';",
  resolveDir: source, loader: 'ts',
}, bundle: true, platform: 'node', format: 'esm', write: false });
const { lookupLatestVersion, readText } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

async function fixture(context) {
  const directory = await mkdtemp(join(tmpdir(), 'frm-settings-reader-'));
  context.after(() => rm(directory, { recursive: true, force: true }));
  return directory;
}

async function bounded(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Settings lookup did not settle')), 1000);
    })]);
  } finally { clearTimeout(timer); }
}

test('settings reads preserve regular empty, JSON and Unicode text', async context => {
  const directory = await fixture(context);
  const file = join(directory, 'settings.json');
  for (const text of ['', '{"autoUpdatesChannel":"stable"}\n', '{"label":"日本語 é"}\n']) {
    await writeFile(file, text);
    assert.equal(await readText(file), text);
    assert.equal(await readFile(file, 'utf8'), text);
  }
});

test('settings reads retain the existing text-length limit', async context => {
  const directory = await fixture(context);
  const file = join(directory, 'settings.json');
  const maximum = 'a'.repeat(1024 * 1024);
  await writeFile(file, maximum);
  assert.equal(await readText(file), maximum);
  await writeFile(file, `${maximum}a`);
  assert.equal(await readText(file), null);
});

test('settings reads return null for absent paths and directories', async context => {
  const directory = await fixture(context);
  assert.equal(await readText(join(directory, 'absent.json')), null);
  assert.equal(await readText(directory), null);
});

test('settings reads follow a regular-file symlink',
  { skip: process.platform === 'win32' }, async context => {
    const directory = await fixture(context);
    const file = join(directory, 'settings.json'), link = join(directory, 'settings-link.json');
    await writeFile(file, '{"autoUpdatesChannel":"stable"}');
    await symlink(file, link);
    assert.equal(await readText(link), await readFile(file, 'utf8'));
  });

test('a settings FIFO cannot hold a cancelled version lookup open',
  { skip: process.platform === 'win32', timeout: 5000 }, async context => {
    const directory = await mkdtemp(join(tmpdir(), 'frm-settings-reader-fifo-'));
    const file = join(directory, 'settings.json');
    let lookup, reading, settled = false;
    context.after(async () => {
      try {
        if (lookup && !settled) {
          const release = await open(file, constants.O_RDWR | constants.O_NONBLOCK);
          await release.close();
          await bounded(lookup);
        }
      } finally {
        try { if (reading) await bounded(reading); }
        finally { await rm(directory, { recursive: true, force: true }); }
      }
    });
    const fifo = spawnSync('mkfifo', ['-m', '600', file], { encoding: 'utf8' });
    assert.equal(fifo.status, 0, fifo.error?.message || fifo.stderr);
    const controller = new AbortController();
    const entered = Promise.withResolvers();
    const effects = [];
    lookup = lookupLatestVersion('claude', {
      kind: 'command', owner: { kind: 'native' },
      command: { command: 'unused-synthetic-client', args: [], display: 'synthetic client' },
    }, { platform: 'linux', home: directory, env: { CLAUDE_CONFIG_DIR: directory },
      signal: controller.signal,
      read: path => {
        if (path !== file) return Promise.resolve(null);
        reading = readText(path);
        entered.resolve();
        return reading;
      },
      fetch: async () => { effects.push('fetch'); return null; },
      runner: { capture: async () => { effects.push('command'); return null; } },
    }).finally(() => { settled = true; });
    await bounded(entered.promise);
    controller.abort(new Error('Synthetic cancellation'));
    assert.equal(await bounded(lookup), null);
    assert.deepEqual(effects, []);
    // Cancellation may settle before the existing read closes its file handle.
    await bounded(reading);
    let readerCheck;
    try {
      readerCheck = await open(file, constants.O_WRONLY | constants.O_NONBLOCK);
      assert.fail('The settings FIFO still has a reader open');
    } catch (error) {
      assert.equal(error.code, 'ENXIO');
    } finally { await readerCheck?.close(); }
  });
