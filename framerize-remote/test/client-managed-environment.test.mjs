import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const source = new URL('../src/compatibility/versions.ts', import.meta.url);
const compiled = await build({ entryPoints: [fileURLToPath(source)], bundle: true,
  platform: 'node', format: 'esm', write: false });
const { claudeUpdateChannel } = await import(`data:text/javascript;base64,${
  Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
console.log(JSON.stringify({ runtime: process.version,
  versions: digest(await readFile(source)),
  conflicts: digest(await readFile(new URL('../src/compatibility/conflicts.ts', import.meta.url))),
  bundle: digest(compiled.outputFiles[0].contents) }));

async function selection(platform, env, managed) {
  const user = join('synthetic-user', 'settings.json');
  const reads = [];
  const channel = await claudeUpdateChannel(async path => {
    reads.push(path);
    if (path === user) return '{"autoUpdatesChannel":"latest"}';
    return path === managed ? '{"autoUpdatesChannel":"stable"}' : null;
  }, platform, env, user);
  return { channel, reads, user };
}

for (const key of ['ProgramFiles', 'PROGRAMFILES', 'pRoGrAmFiLeS']) {
  test(`copied Windows ${key} preserves the managed update channel`, async () => {
    const original = { [key]: 'D:\\Synthetic Applications' };
    const env = { ...original };
    const managed = join(original[key], 'ClaudeCode', 'managed-settings.json');
    const result = await selection('win32', env, managed);
    assert.equal(result.channel, 'stable');
    assert.deepEqual(result.reads, [result.user, managed]);
    assert.deepEqual(env, original);
  });
}

test('missing Windows ProgramFiles retains the default managed directory', async () => {
  const managed = join('C:\\Program Files', 'ClaudeCode', 'managed-settings.json');
  const result = await selection('win32', {}, managed);
  assert.equal(result.channel, 'stable');
  assert.deepEqual(result.reads, [result.user, managed]);
});

test('non-Windows managed settings ignore ProgramFiles', async () => {
  for (const [platform, managed] of [
    ['linux', join('/etc/claude-code', 'managed-settings.json')],
    ['darwin', join('/Library/Application Support/ClaudeCode', 'managed-settings.json')],
  ]) {
    const result = await selection(platform, { ProgramFiles: 'D:\\Synthetic Applications' }, managed);
    assert.equal(result.channel, 'stable');
    assert.deepEqual(result.reads, [result.user, managed]);
  }
});
