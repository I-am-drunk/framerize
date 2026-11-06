import assert from 'node:assert/strict';
import test from 'node:test';
import files from 'node:fs/promises';
import { constants } from 'node:fs';
import { syncBuiltinESMExports } from 'node:module';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const sourceRoot = fileURLToPath(new URL('../src/', import.meta.url));
const compiled = await build({ stdin: { resolveDir: sourceRoot, contents:
  "export { loadConfig, readServerToken } from './config.ts'; export { createProvider } from './provider.ts';" },
  bundle: true, platform: 'node', format: 'esm', write: false });
const { loadConfig, readServerToken, createProvider } = await import(
  `data:text/javascript;base64,${Buffer.from(compiled.outputFiles[0].contents).toString('base64')}`);

for (const kind of ['token', 'provider']) {
  test(`${kind} file rejection does not permit terminal acquisition`,
    { skip: process.platform === 'win32', timeout: 2000 }, async context => {
      const path = '/synthetic-private-credential-device';
      let flags, closes = 0;
      const mocked = context.mock.method(files, 'open', async (requested, options) => {
        assert.equal(requested, path);
        flags = options;
        return { async stat() { return { isFile: () => false }; }, async close() { closes++; } };
      });
      context.after(() => { mocked.mock.restore(); syncBuiltinESMExports(); });
      syncBuiltinESMExports();
      if (kind === 'token') {
        await assert.rejects(readServerToken(loadConfig({ serverTokenFile: path }, {})),
          { message: 'Server token file unavailable' });
      } else {
        const provider = createProvider({ upstream: 'http://127.0.0.1:1', authorizationFile: path });
        context.after(() => provider.close());
        await assert.rejects(provider.forward({ path: '/v1/responses',
          body: (async function* () { yield Buffer.from('{}'); })() }),
        { code: 'credentials_unavailable' });
      }
      assert.equal(closes, 1);
      assert.equal(typeof flags, 'number');
      assert.notEqual(flags & constants.O_NOCTTY, 0, 'File rejection must not acquire a terminal');
    });
}
