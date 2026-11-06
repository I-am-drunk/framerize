import assert from 'node:assert/strict';
import test from 'node:test';
import { access, mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { delimiter, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const packageRoot = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
const { findBrowser } = await import(pathToFileURL(join(packageRoot, 'dist/browser-login.js')).href);
const posix = { skip: process.platform === 'win32' && 'POSIX executable and symlink semantics', timeout: 5000 };

async function fixture(context) {
  const root = await mkdtemp(join(tmpdir(), 'framerize-browser-discovery-'));
  context.after(async () => {
    await rm(root, { recursive: true, force: true });
    await assert.rejects(access(root), { code: 'ENOENT' });
  });
  const shadow = join(root, 'first');
  const bin = join(root, 'second');
  await mkdir(shadow, { mode: 0o700 });
  await mkdir(bin, { mode: 0o700 });
  const executable = join(bin, 'chromium');
  await writeFile(executable, '#!/bin/sh\nexit 0\n', { mode: 0o700 });
  return { root, shadow, executable, env: { PATH: [shadow, bin].join(delimiter) } };
}

test('a searchable directory does not hide a later browser executable', posix, async context => {
  const fixtureState = await fixture(context);
  await mkdir(join(fixtureState.shadow, 'google-chrome'), { mode: 0o700 });
  assert.equal(await findBrowser({ env: fixtureState.env, platform: 'linux' }), fixtureState.executable);
});

for (const selection of ['browser', 'FRAMERIZE_BROWSER']) {
  test(`${selection} rejects a directory without falling back to PATH`, posix, async context => {
    const fixtureState = await fixture(context);
    const options = { env: { ...fixtureState.env }, platform: 'linux' };
    if (selection === 'browser') options.browser = fixtureState.root;
    else options.env.FRAMERIZE_BROWSER = fixtureState.root;
    assert.equal(await findBrowser(options), undefined);
  });
}

test('a symlink to a directory does not hide a later browser executable', posix, async context => {
  const fixtureState = await fixture(context);
  await symlink(fixtureState.root, join(fixtureState.shadow, 'google-chrome'));
  assert.equal(await findBrowser({ env: fixtureState.env, platform: 'linux' }), fixtureState.executable);
});

test('a symlink to an executable remains a valid browser candidate', posix, async context => {
  const fixtureState = await fixture(context);
  const linked = join(fixtureState.shadow, 'google-chrome');
  await symlink(fixtureState.executable, linked);
  assert.equal(await findBrowser({ env: fixtureState.env, platform: 'linux' }), linked);
  assert.equal(await findBrowser({ browser: linked, env: {}, platform: 'linux' }), linked);
});

test('a non-executable regular file does not hide a later browser', posix, async context => {
  const fixtureState = await fixture(context);
  await writeFile(join(fixtureState.shadow, 'google-chrome'), 'synthetic non-executable', { mode: 0o600 });
  assert.equal(await findBrowser({ env: fixtureState.env, platform: 'linux' }), fixtureState.executable);
});

test('explicit browser selection still wins over environment and PATH', posix, async context => {
  const fixtureState = await fixture(context);
  const preferred = join(fixtureState.root, 'preferred-browser');
  await writeFile(preferred, '#!/bin/sh\nexit 0\n', { mode: 0o700 });
  const env = { ...fixtureState.env, FRAMERIZE_BROWSER: preferred };
  assert.equal(await findBrowser({ env, platform: 'linux' }), preferred);
  assert.equal(await findBrowser({ browser: fixtureState.executable, env, platform: 'linux' }), fixtureState.executable);
});
