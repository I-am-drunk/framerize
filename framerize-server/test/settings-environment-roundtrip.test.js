import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createHash } from 'node:crypto';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: 'export * from "./settings.ts"; export { loadConfig } from "./config.ts";',
  resolveDir: fileURLToPath(new URL('../src/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
const bundle = compiled.outputFiles[0].contents;
const { captureSettings, readSettings, configuredSettings, loadConfig } = await import(
  `data:text/javascript;base64,${Buffer.from(bundle).toString('base64')}`);
console.log(JSON.stringify({ node: process.version, platform: process.platform,
  inputs: Object.keys(compiled.metafile.inputs).sort(),
  bundleSha256: createHash('sha256').update(bundle).digest('hex') }));

for (const role of ['server', 'upstream', 'extra']) {
  for (const name of ['__proto__', 'constructor', 'SYNTHETIC_SESSION']) {
    test(`captured ${role} environment preserves ${name} after reload`, { timeout: 3000 }, async context => {
      const root = await mkdtemp(join(tmpdir(), 'framerize-env-roundtrip-'));
      const cleanup = () => rm(root, { recursive: true, force: true });
      context.after(cleanup);
      const file = join(root, 'server.json');
      const value = 'synthetic-roundtrip-token';
      const env = Object.freeze(Object.fromEntries([[name, value], ['UNRELATED', 'not-captured']]));
      const before = Object.entries(env);
      let captured, decoded, loaded, reloadError;
      try {
        const options = role === 'server' ? { serverTokenEnv: name }
          : { serverToken: '', ...(role === 'upstream' ? { authorizationEnv: name } : {}) };
        const config = loadConfig({ backend: 'http', host: '127.0.0.1', port: 0, ...options }, env);
        if (role === 'server') assert.equal(config.serverToken, value);
        captured = captureSettings(config, env, role === 'extra' ? [name] : []);
        const contents = `${JSON.stringify(captured, null, 2)}\n`;
        await writeFile(file, contents, { mode: 0o600 });
        decoded = await readSettings(file);
        try { loaded = await configuredSettings(file, {}, {}); }
        catch (error) { reloadError = error.message; }
        assert.equal(await readFile(file, 'utf8'), contents);
        assert.deepEqual(Object.entries(env), before);
        assert.equal(Object.getPrototypeOf(env), Object.prototype);
      } finally { await cleanup(); }
      await assert.rejects(readFile(file), { code: 'ENOENT' });
      context.diagnostic(JSON.stringify({ role, name,
        capturedNames: Object.keys(captured.environment),
        serializedNames: Object.keys(decoded.environment), reloadError,
        stateRemoved: true }));
      assert.equal(Object.hasOwn(captured.environment, name), true,
        'Captured environment must retain a literal own property for the configured name');
      assert.deepEqual(Object.keys(captured.environment), [name]);
      assert.equal(captured.environment[name], value);
      assert.equal(Object.hasOwn(decoded.environment, name), true);
      assert.equal(decoded.environment[name], value);
      assert.equal(reloadError, undefined);
      assert.equal(loaded.env[name], value);
      if (role === 'server') assert.equal(loaded.config.serverToken, value);
      if (role === 'upstream') assert.equal(loaded.config.authorizationEnv, name);
    });
  }
}
