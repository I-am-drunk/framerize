import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { checkPackageCoexistence, installedBytes } from '../scripts/package-coexistence.mjs';

function fixture(context, name, { crlf = false, probe = '', ...overrides } = {}) {
  const directory = mkdtempSync(path.join(tmpdir(), 'coexistence fixture '));
  context.after(() => rmSync(directory, { recursive: true, force: true }));
  const root = path.join(directory, 'package');
  mkdirSync(path.join(root, 'bin'), { recursive: true });
  mkdirSync(path.join(root, 'src'));
  const server = name === 'framerize-server';
  const files = {
    'package.json': JSON.stringify({ name, version: '0.0.1', type: 'module',
      bin: { [name]: `./bin/${name}.js` },
      exports: { '.': './src/server.js', './config': './src/config.js',
        ...(server ? { './accounts': './src/accounts.js', './logger': './src/logger.js',
          './provider': './src/provider.js' } : {}) }, ...overrides }),
    'src/server.js': 'export function createServer() {}\n',
    'src/config.js': `export function ${server ? 'loadConfig' : 'parseConfig'}() {
      return { port: ${server ? 8789 : 8788} };
    }\n`,
    [`bin/${name}.js`]: `#!/usr/bin/env node
import assert from 'node:assert/strict';
assert.equal(process.env.FRAMER_ACCOUNTS, undefined);
assert.equal(process.env.FRAMER_UPSTREAM, undefined);
assert.equal(process.env.FRAMER_HOST, undefined);
assert.equal(process.env.FRAMER_PORT, undefined);
assert.equal(process.env.FRAMERIZE_SERVER_TOKEN, undefined);
assert.equal(process.env.UPSTREAM_TOKEN, undefined);
assert.equal(process.env.HTTP_PROXY, undefined);
${probe}
console.log(${JSON.stringify(server ? 'Framerize server' : 'Framerize Nexus')});
`,
  };
  if (server) {
    for (const name of ['accounts', 'logger', 'provider']) files[`src/${name}.js`] = 'export {};\n';
  }
  for (const [relative, content] of Object.entries(files)) {
    writeFileSync(path.join(root, relative), crlf ? content.replace(/\n/g, '\r\n') : content,
      { mode: relative.startsWith('bin/') ? 0o755 : 0o644 });
  }
  const tarball = path.join(directory, `${name}.tgz`);
  execFileSync('tar', ['-czf', tarball, '-C', directory, 'package']);
  return tarball;
}

for (const crlf of [false, true]) {
  test(`installs the exact pair offline with ${crlf ? 'CRLF' : 'LF'} source bytes`, (context) => {
    const nexus = fixture(context, 'framerize-nexus', { crlf,
      scripts: { postinstall: 'node -e "process.exit(99)"' } });
    const server = fixture(context, 'framerize-server', { crlf });
    const before = [nexus, server].map((file) => readFileSync(file));
    const poisoned = {
      FRAMER_ACCOUNTS: '/private/never-read.json', FRAMER_UPSTREAM: 'https://never.invalid',
      FRAMER_HOST: '0.0.0.0', FRAMER_PORT: '9999', FRAMERIZE_SERVER_TOKEN: 'never-inherit',
      UPSTREAM_TOKEN: 'never-inherit', HTTP_PROXY: 'http://never.invalid',
      NODE_OPTIONS: '--import=/never-import.mjs', npm_config_userconfig: '/never-read.npmrc',
    };
    const saved = Object.fromEntries(Object.keys(poisoned).map((key) => [key, process.env[key]]));
    let evidence;
    try {
      Object.assign(process.env, poisoned);
      evidence = checkPackageCoexistence([path.relative(process.cwd(), nexus), server]);
    } finally {
      for (const [key, value] of Object.entries(saved)) {
        if (value === undefined) delete process.env[key];
        else process.env[key] = value;
      }
    }
    assert.equal(evidence.node, process.version);
    assert.deepEqual(evidence.packages.map(({ name }) => name), ['framerize-nexus', 'framerize-server']);
    assert.deepEqual(evidence.packages.map(({ sha256 }) => sha256), before.map((bytes) =>
      createHash('sha256').update(bytes).digest('hex')));
    assert.deepEqual([nexus, server].map((file) => readFileSync(file)), before);
  });
}

test('only a launcher shebang may lose CR during installation', () => {
  const original = Buffer.from('#!/usr/bin/env node\r\nbody\r\n');
  assert.deepEqual(installedBytes(original, true), Buffer.from('#!/usr/bin/env node\nbody\r\n'));
  assert.deepEqual(installedBytes(original, false), original);
  const plain = Buffer.from('body\r\n');
  assert.deepEqual(installedBytes(plain, true), plain);
});

test('requires two distinct tarballs with the expected names', (context) => {
  const nexus = fixture(context, 'framerize-nexus');
  assert.throws(() => checkPackageCoexistence([nexus]), /exactly two/);
  assert.throws(() => checkPackageCoexistence([nexus, nexus]), /must be distinct/);
  const duplicate = fixture(context, 'framerize-nexus');
  assert.throws(() => checkPackageCoexistence([nexus, duplicate]), /one Nexus and one server/);
});

test('rejects unexpected package dependencies before installation', (context) => {
  const nexus = fixture(context, 'framerize-nexus', { dependencies: { unexpected: '1.0.0' } });
  const server = fixture(context, 'framerize-server');
  assert.throws(() => checkPackageCoexistence([nexus, server]), /has dependencies/);
});

test('rejects a conflicting bin name before installation', (context) => {
  const nexus = fixture(context, 'framerize-nexus', { bin: { 'framerize-server': 'bin/framerize-nexus.js' } });
  const server = fixture(context, 'framerize-server');
  assert.throws(() => checkPackageCoexistence([nexus, server]), /unexpected executable name/);
});

test('requires every current server export to resolve', (context) => {
  const nexus = fixture(context, 'framerize-nexus');
  const server = fixture(context, 'framerize-server', {
    exports: { '.': './src/server.js', './config': './src/config.js' },
  });
  assert.throws(() => checkPackageCoexistence([server, nexus]), /ERR_PACKAGE_PATH_NOT_EXPORTED/);
});

for (const probe of [
  "(await import('node:net')).connect({host:'127.0.0.1',port:1});",
  "(await import('node:net')).createServer().listen(0,'127.0.0.1');",
  "await fetch('http://127.0.0.1:1');",
]) {
  test(`blocks fixture network calls: ${probe}`, (context) => {
    const nexus = fixture(context, 'framerize-nexus', { probe });
    const server = fixture(context, 'framerize-server');
    assert.throws(() => checkPackageCoexistence([nexus, server]), /Network disabled/);
  });
}
