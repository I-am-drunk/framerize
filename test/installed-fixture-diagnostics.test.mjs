import assert from 'node:assert/strict';
import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { startInstalledServer } from '../.tools/scripts/fixtures/installed-server.js';

async function fixture(context, script) {
  const root = await mkdtemp(join(tmpdir(), 'installed-diagnostics-'));
  context.after(() => rm(root, { recursive: true, force: true }));
  const bin = join(root, 'node_modules/framerize-server/bin');
  await mkdir(bin, { recursive: true });
  await writeFile(join(bin, 'framerize-server.js'), script);
  return { root, env: { HOME: root, USERPROFILE: root, PATH: '', NODE_DISABLE_COMPILE_CACHE: '1' } };
}

test('startup failure retains child output and the original error', { timeout: 10000 }, async context => {
  const state = await fixture(context, `
    process.stdout.write('synthetic startup detail\\n');
    process.stderr.write('synthetic startup stderr\\n');
    process.exit(23);
  `);
  await assert.rejects(startInstalledServer(state.root, state.env, 'http://127.0.0.1:1'), error => {
    assert.match(error.message, /Installed server failed before listening/);
    assert.match(error.message, /synthetic startup detail/);
    assert.match(error.message, /synthetic startup stderr/);
    assert.ok(error.cause instanceof Error);
    return true;
  });
});

for (const exitCode of [0, 23]) {
  test(`shutdown exit ${exitCode} preserves the fixture contract`, {
    timeout: 10000,
    skip: process.platform === 'win32' && exitCode !== 0
      ? 'Windows SIGTERM cannot run the synthetic shutdown handler' : false,
  }, async context => {
    const state = await fixture(context, `
      process.on('SIGTERM', () => {
        require('node:fs').writeSync(1, 'synthetic shutdown detail\\n');
        process.exit(${exitCode});
      });
      setInterval(() => {}, 1000);
      process.stderr.write(JSON.stringify({event: 'server.listening', port: 1}) + '\\n');
    `);
    const server = await startInstalledServer(state.root, state.env, 'http://127.0.0.1:1');
    try {
      const closing = server.close();
      assert.equal(server.close(), closing, 'Closing must retain its original result');
      if (exitCode === 0) await closing;
      else await assert.rejects(closing, error => {
        assert.match(error.message, /Installed server exit/);
        assert.match(error.message, /"code":23/);
        assert.match(error.message, /synthetic shutdown detail/);
        return true;
      });
    } finally { await server.close().catch(() => {}); }
  });
}
