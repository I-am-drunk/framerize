import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { build } from 'esbuild';

const compiled = await build({ stdin: {
  contents: "export { runSetup } from './cmdSetup.ts'; export { loadConfigFile } from './configStore.ts'; export { applyListenOverrides } from './listenOptions.ts'; export { normalizeHostname } from './hostPolicy.ts';",
  resolveDir: fileURLToPath(new URL('../src/', import.meta.url)),
}, bundle: true, platform: 'node', format: 'esm', write: false });
const bytes = compiled.outputFiles[0].contents;
const bundleHash = createHash('sha256').update(bytes).digest('hex');
const { runSetup, loadConfigFile, applyListenOverrides, normalizeHostname } = await import(
  `data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);

const cases = [
  ['FE80::A%FixtureNIC', 'fe80::a%FixtureNIC'],
  ['[FE80::B%FixtureNIC.7]', 'fe80::b%FixtureNIC.7'],
  ['fe80::c%fixture.', 'fe80::c%fixture.'],
  ['FE80::D%42', 'fe80::d%42'],
  ['LOCALHOST.', 'localhost'],
  ['[2001:DB8::A]', '2001:db8::a'],
];

for (const [host, expected] of cases) {
  test(`normalizing ${host} preserves its scope spelling`, { timeout: 3000 }, async context => {
    const home = await mkdtemp(join(tmpdir(), 'frm-scoped-host-'));
    context.after(async () => {
      await rm(home, { recursive: true, force: true });
      await assert.rejects(readFile(join(home, 'profile.json')), { code: 'ENOENT' });
    });
    const path = join(home, 'profile.json');
    const remote = 'http://127.0.0.1:9';
    const args = selected => ['--remote', remote, '--host', selected, '--port', '0'];
    await runSetup(args(host), path, {});
    const saved = JSON.parse(await readFile(path, 'utf8'));
    const loaded = await loadConfigFile(path);
    const overridden = applyListenOverrides({ ...loaded, host: '127.0.0.1' }, { host });
    const observed = { direct: normalizeHostname(host), saved: saved.host,
      loaded: loaded.host, overridden: overridden.host };
    await runSetup(args('127.0.0.1'), path, {});
    const recovered = await loadConfigFile(path);
    assert.equal(recovered.host, '127.0.0.1');
    assert.equal(recovered.remote, remote);
    assert.equal(recovered.port, 0);
    context.diagnostic(JSON.stringify({ node: process.version, bundleHash, host, observed, recovered: true }));
    assert.deepEqual(observed, { direct: expected, saved: expected,
      loaded: expected, overridden: expected });
    assert.equal(loaded.remote, remote);
    assert.equal(loaded.port, 0);
  });
}
