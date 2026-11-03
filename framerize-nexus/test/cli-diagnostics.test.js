import test from 'node:test';
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const entry = fileURLToPath(new URL('../bin/framerize-nexus.js', import.meta.url));
const secret = 'SYNTHETIC_ARGV_SECRET_8e41';
const privatePath = `/synthetic/private/${secret}`;
const guard = `
import net from 'node:net';
import http from 'node:http';
import https from 'node:https';
import { syncBuiltinESMExports } from 'node:module';
const forbidden = () => { process.stderr.write('unexpected fixture network'); process.exit(97); };
net.Server.prototype.listen = forbidden;
net.Socket.prototype.connect = forbidden;
http.request = http.get = https.request = https.get = globalThis.fetch = forbidden;
syncBuiltinESMExports();
`;
const preload = `data:text/javascript,${encodeURIComponent(guard)}`;

function execute(context, argumentsList) {
  const directory = mkdtempSync(path.join(tmpdir(), 'nexus-diagnostics-'));
  context.after(() => rmSync(directory, { recursive: true, force: true }));
  return spawnSync(process.execPath, ['--import', preload, ...argumentsList], {
    cwd: directory,
    encoding: 'utf8',
    timeout: 5000,
    env: {
      PATH: path.dirname(process.execPath),
      ...(process.env.SystemRoot ? { SystemRoot: process.env.SystemRoot } : {}),
      HOME: directory,
      USERPROFILE: directory,
      XDG_CONFIG_HOME: directory,
      TMPDIR: directory,
      TEMP: directory,
      TMP: directory,
    },
  });
}

const cases = [
  ['raw-token option', ['check', `--remote-token=${secret}`], 'unknown option'],
  ['private-path option', ['check', `--env-file=${privatePath}`], 'unknown option'],
  ['secret-bearing command', [secret], 'unknown command'],
  ['path-bearing command', [privatePath], 'unknown command'],
  ['leading raw-token option', [`--remote-token=${secret}`], 'unknown command'],
];

for (const [name, argumentsList, message] of cases) {
  test(`CLI does not echo ${name}`, (context) => {
    const result = execute(context, ['--', entry, ...argumentsList]);
    assert.equal(result.error, undefined);
    assert.equal(result.signal, null);
    assert.equal(result.status, 1);
    assert.equal(result.stdout, '');
    assert.deepEqual(JSON.parse(result.stderr), { event: 'fatal', message });
    assert.equal(result.stderr.includes(secret), false);
    assert.equal(result.stderr.includes(privatePath), false);
  });
}

test('known missing-value errors retain only the allowlisted option name', (context) => {
  const result = execute(context, ['--', entry, 'check', '--upstream']);
  assert.equal(result.status, 1);
  assert.equal(result.stdout, '');
  assert.deepEqual(JSON.parse(result.stderr), {
    event: 'fatal', message: '--upstream needs a value',
  });
});

for (const operation of ['server', 'request']) {
  test(`diagnostic fixture blocks ${operation} networking`, (context) => {
    const source = operation === 'server'
      ? "(await import('node:http')).createServer().listen(0, '127.0.0.1')"
      : "(await import('node:http')).request('http://127.0.0.1:1')";
    const result = execute(context, ['--input-type=module', '--eval', source]);
    assert.equal(result.status, 97);
    assert.equal(result.stderr, 'unexpected fixture network');
  });
}
