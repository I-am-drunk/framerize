import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import https from 'node:https';
import childProcess from 'node:child_process';
import { syncBuiltinESMExports } from 'node:module';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';
import test from 'node:test';
import { build } from 'esbuild';

const packageRoot = resolve(process.env.FRAMERIZE_REMOTE_SOURCE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url)));
const sourceRoot = resolve(packageRoot, '..');
const compiled = await build({ absWorkingDir: sourceRoot,
  entryPoints: [join(packageRoot, 'src/frmCli.ts')],
  alias: { '@framerize/core': join(sourceRoot, 'framerize-core/src/index.ts') },
  bundle: true, platform: 'node', format: 'esm', write: false, metafile: true });
const hash = bytes => createHash('sha256').update(bytes).digest('hex');
const inputs = await Promise.all(Object.keys(compiled.metafile.inputs).sort()
  .map(async path => ({ path, sha256: hash(await fs.readFile(join(sourceRoot, path))) })));
const bytes = compiled.outputFiles[0].contents;
const { runFrm } = await import(`data:text/javascript;base64,${Buffer.from(bytes).toString('base64')}`);
console.log(JSON.stringify({ node: process.version, platform: process.platform,
  inputs, bundleSha256: hash(bytes) }));

for (const scenario of ['loop', 'missing']) {
  test(`bare frm distinguishes a ${scenario} profile from first use`, {
    timeout: 3000, skip: scenario === 'loop' && process.platform === 'win32',
  }, async context => {
    const root = await fs.mkdtemp(join(tmpdir(), 'frm-bare-profile-'));
    const profile = join(root, 'framerize-remote.json');
    const envNames = ['FRM_HOME', 'FRAMERIZE_REMOTE_HOME', 'FRM_REMOTE'];
    const originalEnv = envNames.map(name => [name, process.env[name]]);
    let cleanupPromise;
    const cleanup = () => cleanupPromise ??= (async () => {
      context.mock.restoreAll();
      syncBuiltinESMExports();
      for (const [name, value] of originalEnv) {
        if (value === undefined) delete process.env[name];
        else process.env[name] = value;
      }
      await fs.rm(root, { recursive: true, force: true });
      await assert.rejects(fs.stat(root), { code: 'ENOENT' });
    })();
    context.after(cleanup);
    process.env.FRM_HOME = root;
    delete process.env.FRAMERIZE_REMOTE_HOME;
    delete process.env.FRM_REMOTE;
    const canary = join(root, 'canary');
    await fs.writeFile(canary, 'synthetic-private-state', { mode: 0o600 });
    if (scenario === 'loop') await fs.symlink('framerize-remote.json', profile);
    await assert.rejects(fs.access(profile), { code: scenario === 'loop' ? 'ELOOP' : 'ENOENT' });
    let boundaryCalls = 0;
    const forbidden = () => { boundaryCalls++; throw new Error('Unexpected external operation'); };
    for (const transport of [http, https]) context.mock.method(transport, 'request', forbidden);
    for (const name of ['spawn', 'execFile']) context.mock.method(childProcess, name, forbidden);
    syncBuiltinESMExports();
    const stdout = [], stderr = [];
    const result = await runFrm([], { stdout: text => stdout.push(text),
      stderr: text => stderr.push(text) }, { program: [process.execPath, 'unused-launcher'] })
      .then(value => ({ value }), error => ({ error }));
    assert.equal(await fs.readFile(canary, 'utf8'), 'synthetic-private-state');
    if (scenario === 'loop') assert.equal(await fs.readlink(profile), 'framerize-remote.json');
    else await assert.rejects(fs.lstat(profile), { code: 'ENOENT' });
    await cleanup();
    context.diagnostic(JSON.stringify({ scenario, code: result.error?.code,
      boundaryCalls, stateRemoved: true }));
    assert.equal(boundaryCalls, 0);
    assert.deepEqual(stdout, []);
    assert.deepEqual(stderr, []);
    assert.equal(result.error?.code, scenario === 'loop' ? 'profile_invalid' : 'remote_required',
      'Only an absent profile should be treated as a first run');
  });
}
