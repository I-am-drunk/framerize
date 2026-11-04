import assert from 'node:assert/strict';
import { copyFile, lstat, mkdtemp, readFile, readdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';

const root = fileURLToPath(new URL('../', import.meta.url));
const npm = process.env.npm_execpath;
if (!npm) throw new Error('Run this check with npm run pack:check');
const directory = await mkdtemp(path.join(tmpdir(), 'framerize-server-pack-'));
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
  const installed = path.join(directory, 'node_modules/framerize-server');
  const manifest = JSON.parse(await readFile(path.join(installed, 'package.json')));
  if (manifest.name !== 'framerize-server' || manifest.version !== source.version ||
      manifest.private !== true || ['dependencies', 'optionalDependencies', 'peerDependencies']
        .some((field) => Object.keys(manifest[field] ?? {}).length)) {
    throw new Error('Unexpected package dependencies');
  }
  const normalizeManifest = (value) => ({ ...value, bin: Object.fromEntries(
    Object.entries(value.bin ?? {}).map(([name, target]) => [name, target.replace(/^\.\//, '')]),
  ) });
  assert.deepEqual(normalizeManifest(manifest), normalizeManifest(source),
    'Installed package metadata differs');
  const files = ['README.md', 'LICENSE',
    'bin/framerize-server.js', ...['core.js', 'core.d.ts', 'accounts.js', 'accounts.d.ts',
      'backend.js', 'backend.d.ts', 'castle.js', 'castle-loader.js', 'castle-loader.d.ts',
      'account-backend.js', 'account-backend.d.ts',
      'config.js', 'config.d.ts', 'errors.js', 'errors.d.ts', 'provider.js', 'provider.d.ts',
      'framer-auth.js', 'framer-auth.d.ts',
      'framer-backend.js', 'framer-backend.d.ts', 'framer-request.js', 'framer-request.d.ts',
      ...['completed-reply', 'completed-stream', 'events', 'messages-stream', 'responses-stream']
        .flatMap(name => [`compat/${name}.js`, `compat/${name}.d.ts`]),
      'cli.js', 'cli.d.ts', 'logger.js', 'logger.d.ts', 'server.js', 'server.d.ts',
      ...['backend', 'http-policy', 'instance', 'instance-options', 'readiness'].map((name) => `core-types/${name}.d.ts`)]
      .map((name) => `dist/${name}`)];
  const directories = new Set(files.flatMap(file => {
    const parts = file.split('/');
    return parts.slice(0, -1).map((_part, index) => parts.slice(0, index + 1).join('/'));
  }));
  assert.deepEqual((await readdir(installed, { recursive: true }))
    .map((name) => name.split(path.sep).join('/')).sort(),
    [...directories, 'package.json', ...files].sort(), 'Unexpected installed package files');
  for (const name of ['package.json', ...files]) {
    assert.ok((await lstat(path.join(installed, name))).isFile(), 'Unexpected installed package file type');
  }
  for (const name of files) {
    let expected = process.argv.includes('--downloaded') && !name.startsWith('dist/')
      ? execFileSync('git', ['show', `HEAD:framerize-server/${name}`], { cwd: root })
      : await readFile(path.join(root, name));
    const newline = expected.indexOf(10);
    if (name === 'bin/framerize-server.js' && expected[0] === 35 && expected[1] === 33 &&
        expected[newline - 1] === 13) {
      expected = Buffer.concat([expected.subarray(0, newline - 1), expected.subarray(newline)]);
    }
    assert.deepEqual(await readFile(path.join(installed, name)),
      expected, `Installed source differs: ${name}`);
  }
  execFileSync(process.execPath, [path.join(directory, 'node_modules/framerize-server/bin/framerize-server.js'), '--help'], {
    cwd: directory, stdio: 'pipe',
  });
  execFileSync(process.execPath, ['--input-type=module', '-e', `
    import assert from 'node:assert/strict';
    import { createServer } from 'framerize-server';
    import { loadConfig } from 'framerize-server/config';
    import { loadAccounts } from 'framerize-server/accounts';
    const accounts = await loadAccounts();
    assert.deepEqual(accounts, []);
    assert.equal(Object.isFrozen(accounts), true);
    const provider = { ready: async () => true, async forward() {
      return { status: 200, headers: {'content-type':'application/json'}, body: [Buffer.from('{}')] };
    } };
    const app = await createServer(loadConfig({host:'127.0.0.1',port:0,upstream:'',serverToken:'package-fixture'}), {provider});
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
