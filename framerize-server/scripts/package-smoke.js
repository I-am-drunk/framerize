import assert from 'node:assert/strict';
import { copyFile, mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const root = fileURLToPath(new URL('../', import.meta.url));
const directory = await mkdtemp(path.join(tmpdir(), 'framerize-server-pack-'));
const npm = process.env.npm_execpath;
if (!npm) throw new Error('Run this check with npm run pack:check');
const run = (args) => execFileSync(process.execPath, [npm, ...args], {
  cwd: directory, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'],
});
try {
  const source = JSON.parse(await readFile(path.join(root, 'package.json'), 'utf8'));
  if (process.argv.includes('--downloaded')) {
    const name = `${source.name}-${source.version}.tgz`;
    await copyFile(path.join(root, name), path.join(directory, name));
  } else {
    run(['pack', root, '--ignore-scripts', '--pack-destination', directory]);
  }
  const tarball = (await readdir(directory)).find((name) => name.endsWith('.tgz'));
  if (!tarball) throw new Error('Package tarball missing');
  run(['install', '--ignore-scripts', '--no-audit', '--no-fund', '--offline', path.join(directory, tarball)]);
  const manifest = JSON.parse(await readFile(path.join(directory, 'node_modules/framerize-server/package.json')));
  if (manifest.name !== 'framerize-server' || manifest.version !== source.version ||
      manifest.private !== true || Object.keys(manifest.dependencies ?? {}).length) {
    throw new Error('Unexpected package dependencies');
  }
  for (const name of ['README.md', 'LICENSE', ...await readdir(path.join(root, 'src')).then((files) =>
    files.map((file) => `src/${file}`)), 'bin/framerize-server.js']) {
    const expected = process.argv.includes('--downloaded')
      ? execFileSync('git', ['show', `HEAD:framerize-server/${name}`], { cwd: root })
      : await readFile(path.join(root, name));
    assert.deepEqual(await readFile(path.join(directory, 'node_modules/framerize-server', name)),
      expected, `Installed source differs: ${name}`);
  }
  execFileSync(process.execPath, [path.join(directory, 'node_modules/framerize-server/bin/framerize-server.js'), '--help'], {
    cwd: directory, stdio: 'pipe',
  });
  execFileSync(process.execPath, ['--input-type=module', '-e', `
    import assert from 'node:assert/strict';
    import { createServer } from 'framerize-server';
    import { loadConfig } from 'framerize-server/config';
    const provider = { ready: async () => true, async forward() {
      return { status: 200, headers: {'content-type':'application/json'}, body: [Buffer.from('{}')] };
    } };
    const app = await createServer(loadConfig({port:0,serverToken:'package-fixture'}), {provider});
    try {
      const address = await app.listen();
      const base = 'http://127.0.0.1:' + address.port;
      assert.equal((await fetch(base + '/health')).status, 200);
      assert.equal((await fetch(base + '/ready')).status, 401);
      const headers = {authorization:'Bearer package-fixture'};
      const ready = await fetch(base + '/ready', {headers});
      assert.equal((await ready.json()).apiVersion, 1);
      const response = await fetch(base + '/v1/responses', {method:'POST',headers,body:'{}'});
      assert.equal(response.status, 200);
      assert.equal(await response.text(), '{}');
    } finally { await app.close(); }
  `], { cwd: directory, stdio: 'inherit' });
  process.stdout.write(`Packed install and loopback smoke passed: ${manifest.name}@${manifest.version}\n`);
} finally { await rm(directory, { recursive: true, force: true }); }
