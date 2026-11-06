import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  resolveDir: fileURLToPath(new URL('../', import.meta.url)),
  contents: `export { runSetup } from './src/cmdSetup.ts';
    export { parsePersisted } from './src/config.ts';
    export { loadConfigFile } from './src/configStore.ts';`,
}, bundle: true, platform: 'node', format: 'esm', write: false });
const bytes = compiled.outputFiles[0].contents;
const bundleSha256 = createHash('sha256').update(bytes).digest('hex');
const { runSetup, parsePersisted, loadConfigFile } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
const remote = 'http://127.0.0.1:9';
const args = host => ['--remote', remote, '--host', host, '--port', '0'];
const existing = '{ "remote": "http://127.0.0.1:8", "host": "127.0.0.1", "port": 8788 }\n';
const contents = path => readFile(path, 'utf8').catch(error => {
  if (error.code === 'ENOENT') return null;
  throw error;
});

for (const host of ['%6cocalhost', 'local%68ost', '127%2e0%2e0%2e1', 'localhost%2e']) {
  test(`setup rejects escaped DNS host ${host} without changing profiles`, { timeout: 3000 }, async context => {
    const home = await mkdtemp(join(tmpdir(), 'frm-escaped-host-'));
    context.after(() => rm(home, { recursive: true, force: true }));
    let parsedError;
    try { parsePersisted({ remote, host }); } catch (error) { parsedError = error; }
    const results = [];
    for (const saved of [false, true]) {
      const path = join(home, saved ? 'existing.json' : 'new.json');
      if (saved) await writeFile(path, existing, { mode: 0o600 });
      const error = await runSetup(args(host), path, {}).then(() => undefined, failure => failure);
      const after = await contents(path);
      await runSetup(args('localhost'), path, {});
      const recovered = await loadConfigFile(path);
      assert.equal(recovered.host, 'localhost');
      assert.equal(recovered.remote, remote);
      assert.equal(recovered.port, 0);
      results.push({ saved, error, after });
    }
    context.diagnostic(JSON.stringify({ node: process.version, bundleSha256, host,
      outcomes: results.map(result => ({ saved: result.saved, code: result.error?.code ?? null,
        profileChanged: result.after !== (result.saved ? existing : null) })), recovered: true }));
    assert.equal(parsedError?.code, 'listen_invalid');
    for (const result of results) {
      assert.equal(result.error?.code, 'listen_invalid');
      assert.equal(result.after, result.saved ? existing : null);
    }
  });
}

for (const host of ['localhost', 'bücher.example', '127.1', 'fe80::1%synthetic0']) {
  test(`setup preserves accepted literal host ${host}`, { timeout: 3000 }, async context => {
    const home = await mkdtemp(join(tmpdir(), 'frm-literal-host-'));
    context.after(() => rm(home, { recursive: true, force: true }));
    const path = join(home, 'profile.json');
    assert.equal(parsePersisted({ remote, host }).host, host);
    await runSetup(args(host), path, {});
    const loaded = await loadConfigFile(path);
    assert.equal(loaded.host, host);
    assert.equal(loaded.port, 0);
    assert.equal(loaded.remote, remote);
    context.diagnostic(JSON.stringify({ node: process.version, bundleSha256, host, saved: true }));
  });
}
