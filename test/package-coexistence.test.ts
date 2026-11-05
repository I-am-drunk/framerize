import test, { type TestContext } from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { checkPackageCoexistence, installedBytes } from '../scripts/package-coexistence.js';

function fixture(context: TestContext, name: string,
  { crlf = false, probe = '', executable = `bin/${name}.js`, legacyExecutable, ...overrides }:
    { crlf?: boolean; probe?: string; executable?: string; legacyExecutable?: string; [key: string]: unknown } = {}) {
  const directory = mkdtempSync(path.join(tmpdir(), 'coexistence fixture '));
  context.after(() => rmSync(directory, { recursive: true, force: true }));
  const root = path.join(directory, 'package');
  mkdirSync(path.join(root, 'bin'), { recursive: true });
  mkdirSync(path.join(root, 'src'));
  const server = name === 'framerize-server';
  const files: Record<string, string> = {
    'package.json': JSON.stringify({ name, version: '0.0.1', type: 'module',
      bin: { [server ? 'framerize' : 'frm']: `./${executable}` },
      exports: { '.': './src/server.js',
        ...(server ? { './config': './src/config.js', './accounts': './src/accounts.js', './logger': './src/logger.js',
          './provider': './src/provider.js' } : {}) }, ...overrides }),
    'src/server.js': server ? 'export function createServer() {}\n' : `
      export function createRemoteBackend() { return { ready() {}, forward() {}, close() {} }; }
      export function parseRemoteEndpoint(origin) { return { origin, loopback: true }; }
    `,
    'src/config.js': `export function ${server ? 'loadConfig' : 'parseConfig'}() {
      return { port: ${server ? 8789 : 8788} };
    }\n`,
    [executable]: `#!/usr/bin/env node
import assert from 'node:assert/strict';
assert.equal(process.env.FRAMER_ACCOUNTS, undefined);
assert.equal(process.env.FRAMER_UPSTREAM, undefined);
assert.equal(process.env.FRAMER_HOST, undefined);
assert.equal(process.env.FRAMER_PORT, undefined);
assert.equal(process.env.FRAMERIZE_SERVER_TOKEN, undefined);
assert.equal(process.env.UPSTREAM_TOKEN, undefined);
assert.equal(process.env.HTTP_PROXY, undefined);
${probe}
console.log(process.argv.includes('version') ? '0.0.1'
  : ${JSON.stringify(server ? 'Server' : 'frm --server-token-env')});
`,
  };
  if (legacyExecutable) files[legacyExecutable] = files[executable]!;
  if (server) {
    for (const name of ['accounts', 'logger', 'provider']) files[`src/${name}.js`] = 'export {};\n';
  }
  for (const [relative, content] of Object.entries(files)) {
    writeFileSync(path.join(root, relative), crlf ? content.replace(/\n/g, '\r\n') : content,
      { mode: relative.startsWith('bin/') ? 0o755 : 0o644 });
  }
  const tarball = path.join(directory, `${name}.tgz`);
  execFileSync('tar', ['--format=ustar', '-czf', path.basename(tarball), 'package'], { cwd: directory });
  return tarball;
}

for (const crlf of [false, true]) {
  test(`installs the exact pair offline with ${crlf ? 'CRLF' : 'LF'} source bytes`, (context) => {
    const remote = fixture(context, 'framerize-remote', { crlf,
      scripts: { postinstall: 'node -e "process.exit(99)"' } });
    const server = fixture(context, 'framerize-server', { crlf });
    const before = [remote, server].map((file) => readFileSync(file));
    const poisoned = {
      FRAMER_ACCOUNTS: '/private/never-read.json', FRAMER_UPSTREAM: 'https://never.invalid',
      FRAMER_HOST: '0.0.0.0', FRAMER_PORT: '9999', FRAMERIZE_SERVER_TOKEN: 'never-inherit',
      UPSTREAM_TOKEN: 'never-inherit', HTTP_PROXY: 'http://never.invalid',
      NODE_OPTIONS: '--import=/never-import.mjs', npm_config_userconfig: '/never-read.npmrc',
    };
    const saved: Record<string, string | undefined> = Object.fromEntries(
      Object.keys(poisoned).map((key) => [key, process.env[key]]));
    let evidence;
    try {
      Object.assign(process.env, poisoned);
      evidence = checkPackageCoexistence([path.relative(process.cwd(), remote), server]);
    } finally {
      for (const [key, value] of Object.entries(saved)) {
        if (value === undefined) delete process.env[key];
        else process.env[key] = value;
      }
    }
    assert.equal(evidence.node, process.version);
    assert.deepEqual(evidence.packages.map(({ name }) => name), ['framerize-remote', 'framerize-server']);
    assert.deepEqual(evidence.packages.map(({ sha256 }) => sha256), before.map((bytes) =>
      createHash('sha256').update(bytes).digest('hex')));
    assert.deepEqual([remote, server].map((file) => readFileSync(file)), before);
  });
}

test('only a launcher shebang may lose CR during installation', () => {
  const original = Buffer.from('#!/usr/bin/env node\r\nbody\r\n');
  assert.deepEqual(installedBytes(original, true), Buffer.from('#!/usr/bin/env node\nbody\r\n'));
  assert.deepEqual(installedBytes(original, false), original);
  const plain = Buffer.from('body\r\n');
  assert.deepEqual(installedBytes(plain, true), plain);
});

test('installs frm with a legacy alias and a canonical launcher filename', context => {
  const remote = fixture(context, 'framerize-remote', {
    crlf: true, executable: 'bin/frm.js',
    bin: { frm: './bin/frm.js', 'framerize-remote': './bin/frm.js' },
  });
  const server = fixture(context, 'framerize-server');
  assert.equal(checkPackageCoexistence([remote, server]).packages.length, 2);
});

test('installs canonical framerize alongside its Server alias and frm', context => {
  const remote = fixture(context, 'framerize-remote');
  const server = fixture(context, 'framerize-server', {
    bin: { framerize: './bin/framerize-server.js', 'framerize-server': './bin/framerize-server.js' },
  });
  assert.equal(checkPackageCoexistence([remote, server]).packages.length, 2);
});

test('requires canonical framerize even when the Server alias exists', context => {
  const remote = fixture(context, 'framerize-remote');
  const server = fixture(context, 'framerize-server', {
    bin: { 'framerize-server': './bin/framerize-server.js' },
  });
  assert.throws(() => checkPackageCoexistence([remote, server]), /unexpected executable name/);
});

test('requires the canonical frm command even when the legacy alias exists', context => {
  const remote = fixture(context, 'framerize-remote', {
    bin: { 'framerize-remote': './bin/framerize-remote.js' },
  });
  const server = fixture(context, 'framerize-server');
  assert.throws(() => checkPackageCoexistence([remote, server]), /unexpected executable name/);
});

test('checks distinct CRLF launchers for frm and its legacy alias', context => {
  const remote = fixture(context, 'framerize-remote', {
    crlf: true, executable: 'bin/frm.js', legacyExecutable: 'bin/framerize-remote.js',
    bin: { frm: './bin/frm.js', 'framerize-remote': './bin/framerize-remote.js' },
  });
  const server = fixture(context, 'framerize-server');
  assert.equal(checkPackageCoexistence([remote, server]).packages.length, 2);
});

test('rejects an alias that points outside the package bin directory', context => {
  const remote = fixture(context, 'framerize-remote', {
    bin: { frm: './bin/framerize-remote.js', 'framerize-remote': '../outside.js' },
  });
  const server = fixture(context, 'framerize-server');
  assert.throws(() => checkPackageCoexistence([remote, server]), /unexpected executable path/);
});

test('requires two distinct tarballs with the expected names', (context) => {
  const remote = fixture(context, 'framerize-remote');
  assert.throws(() => checkPackageCoexistence([remote]), /exactly two/);
  assert.throws(() => checkPackageCoexistence([remote, remote]), /must be distinct/);
  const duplicate = fixture(context, 'framerize-remote');
  assert.throws(() => checkPackageCoexistence([remote, duplicate]), /one Remote and one server/);
});

test('rejects unexpected package dependencies before installation', (context) => {
  const remote = fixture(context, 'framerize-remote', { dependencies: { unexpected: '1.0.0' } });
  const server = fixture(context, 'framerize-server');
  assert.throws(() => checkPackageCoexistence([remote, server]), /has dependencies/);
});

test('rejects a conflicting bin name before installation', (context) => {
  const remote = fixture(context, 'framerize-remote', { bin: { 'framerize-server': 'bin/framerize-remote.js' } });
  const server = fixture(context, 'framerize-server');
  assert.throws(() => checkPackageCoexistence([remote, server]), /unexpected executable name/);
});

test('requires all five baseline server module exports to resolve', (context) => {
  const remote = fixture(context, 'framerize-remote');
  const server = fixture(context, 'framerize-server', {
    exports: { '.': './src/server.js', './config': './src/config.js' },
  });
  assert.throws(() => checkPackageCoexistence([server, remote]), /ERR_PACKAGE_PATH_NOT_EXPORTED/);
});

for (const probe of [
  "(await import('node:net')).connect({host:'127.0.0.1',port:1});",
  "(await import('node:net')).createServer().listen(0,'127.0.0.1');",
  "await fetch('http://127.0.0.1:1');",
]) {
  test(`blocks fixture network calls: ${probe}`, (context) => {
    const remote = fixture(context, 'framerize-remote', { probe });
    const server = fixture(context, 'framerize-server');
    assert.throws(() => checkPackageCoexistence([remote, server]), /Network disabled/);
  });
}
