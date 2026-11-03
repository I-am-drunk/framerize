import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import test from 'node:test';
import { pathToFileURL } from 'node:url';
import { fixtureEnvironment, guardRead, runGuarded } from '../framerize-castle/test/helpers/offline-guard.mjs';

const workflow = fs.readFileSync(new URL('../.github/workflows/library-ci.yml', import.meta.url), 'utf8')
  .replace(/\r\n/g, '\n');

function section(source, name, indent = 0) {
  const lines = source.split('\n');
  const start = lines.indexOf(' '.repeat(indent) + name + ':');
  assert.notEqual(start, -1, 'missing section ' + name);
  const end = lines.findIndex((line, index) => index > start && line.trim() && line.search(/\S/) <= indent);
  return lines.slice(start + 1, end < 0 ? undefined : end).join('\n');
}

function validateWorkflow(source) {
  assert.equal(section(source, 'permissions').trim(), 'contents: read');
  assert.equal([...source.matchAll(/^\s*permissions:/gm)].length, 1);
  assert.match(source, /NODE_OPTIONS: ''/);
  const events = section(source, 'on');
  for (const event of ['push', 'pull_request']) {
    const config = section(events, event, 2);
    for (const entry of ['framerize-castle/**', '.github/workflows/library-ci.yml',
      'test/library-workflow.test.mjs', 'docs/library-ci.md']) assert.ok(config.includes("'" + entry + "'"));
  }
  assert.match(section(events, 'push', 2), /branches: \[main, framer-next\]/);
  assert.match(source, /os: \[ubuntu-22.04, macos-15, windows-2022\]/);
  assert.match(source, /node: \['22\.12\.0', '24'\]/);
  assert.match(source, /runs-on: \$\{\{ matrix.os \}\}/);
  assert.match(source, /node-version: \$\{\{ matrix.node \}\}/);
  assert.match(source, /fail-fast: false/);
  assert.match(source, /timeout-minutes: 10/);
  assert.match(source, /persist-credentials: false/);
  const actions = [...source.matchAll(/uses: (\S+)/g)].map((match) => match[1]);
  assert.deepEqual(actions, [
    'actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683',
    'actions/setup-node@49933ea5288caeca8642d1e84afbd3f7d6820020',
  ]);
  const commands = [...source.matchAll(/^\s*run: (.+)$/gm)].map((match) => match[1]);
  assert.deepEqual(commands, ['node --test test/library-workflow.test.mjs', 'node test/helpers/offline-guard.mjs']);
  assert.match(source, /working-directory: framerize-castle\n        run: node test\/helpers\/offline-guard.mjs/);
  assert.doesNotMatch(source, /pull_request_target|secrets\.|npm |npx |cargo |publish|deploy|continue-on-error|if:|run: [|>]/);
}

test('library workflow preserves offline matrix and read-only policy', () => validateWorkflow(workflow));

for (const [before, after] of [
  ['contents: read', 'contents: write'], ['persist-credentials: false', 'persist-credentials: true'],
  ["      - 'framerize-castle/**'", ''], ["      - 'docs/library-ci.md'", ''],
  ["      - 'test/library-workflow.test.mjs'", ''], ["      - '.github/workflows/library-ci.yml'", ''],
  ['main, framer-next', 'main'], ['macos-15', 'ubuntu-24.04'], ['windows-2022', 'ubuntu-24.04'],
  ["'22.12.0'", "'20'"], ["'24'", "'20'"], ['NODE_OPTIONS: \'\'', 'NODE_OPTIONS: unsafe'],
  ['11bd71901bbe5b1630ceea73d27597364c9af683', 'v4'],
  ['working-directory: framerize-castle', 'working-directory: .'],
  ['node test/helpers/offline-guard.mjs', 'node --test'],
]) test('workflow rejects changed ' + before, () => {
  assert.throws(() => validateWorkflow(workflow.replace(before, after)));
});

function probe(source) {
  const result = runGuarded(['--input-type=module', '--eval', source]);
  assert.equal(result.error, undefined);
  assert.equal(result.status, 0, result.stderr);
  return result.stdout.trim();
}

test('file guard rejects unmanaged capture before the underlying read', () => {
  let calls = 0;
  const read = guardRead(() => { calls += 1; throw new Error('sentinel reached'); }, [path.parse(process.cwd()).root]);
  const file = path.resolve('/tmp/font-probe-capture.json');
  for (const input of [file, Buffer.from(file), pathToFileURL(file), path.join(path.dirname(file), 'unused', '..', path.basename(file))]) {
    assert.throws(() => read(input), { code: 'ERR_FIXTURE_OFFLINE' });
  }
  assert.equal(calls, 0);
});

test('preload denies unmanaged reads through default and named filesystem APIs', () => {
  probe(`import assert from 'node:assert/strict';
    import fs, { readFileSync } from 'node:fs';
    import { readFile } from 'node:fs/promises';
    const file = process.env.HOME + '-absent-' + crypto.randomUUID();
    for (const operation of [() => fs.readFileSync(file), () => readFileSync(file),
      () => fs.readFile(file, () => {}), () => fs.openSync(file, 'r'),
      () => fs.open(file, 'r', () => {}), () => fs.createReadStream(file)]) {
      assert.throws(operation, { code: 'ERR_FIXTURE_OFFLINE' });
    }
    await assert.rejects(readFile(file), { code: 'ERR_FIXTURE_OFFLINE' });
    await assert.rejects(fs.promises.open(file), { code: 'ERR_FIXTURE_OFFLINE' });`);
});

test('launcher discards inherited credentials and realm state before imports, then cleans up', () => {
  const previous = process.env.FRAMERIZE_REALM_STATE;
  process.env.FRAMERIZE_REALM_STATE = '/unmanaged-fixture-state';
  try {
    assert.equal(fixtureEnvironment('fixture').FRAMERIZE_REALM_STATE, path.join('fixture', 'realm-state.json'));
    const directory = probe(`import assert from 'node:assert/strict';
      import fs from 'node:fs'; import path from 'node:path';
      const env = process.env;
      assert.equal(env.FRAMERIZE_REALM_STATE, path.join(env.HOME, 'realm-state.json'));
      assert.equal(fs.existsSync(env.FRAMERIZE_REALM_STATE), false);
      for (const key of Object.keys(env)) assert.ok(!/TOKEN|SECRET|PROXY|COOKIE|CREDENTIAL|SEED_CUID/i.test(key));
      fs.writeFileSync(env.FRAMERIZE_REALM_STATE, '{"fixture":true}');
      assert.deepEqual(JSON.parse(fs.readFileSync(env.FRAMERIZE_REALM_STATE)), { fixture: true });
      console.log(env.HOME);`);
    assert.equal(fs.existsSync(directory), false);
  } finally {
    if (previous === undefined) delete process.env.FRAMERIZE_REALM_STATE;
    else process.env.FRAMERIZE_REALM_STATE = previous;
  }
});

test('ambient repository state is denied without reading it', () => {
  let calls = 0;
  const file = new URL('../framerize-castle/castle/experiments/realm-state.json', import.meta.url);
  const read = guardRead(() => { calls += 1; }, [path.parse(process.cwd()).root]);
  assert.throws(() => read(file), { code: 'ERR_FIXTURE_OFFLINE' });
  assert.equal(calls, 0);
});

test('failed fixture processes also remove their private directory', () => {
  const result = runGuarded(['--eval', "console.log(process.env.HOME); process.exitCode = 7"]);
  assert.equal(result.status, 7);
  assert.equal(fs.existsSync(result.stdout.trim()), false);
});

test('symlinked temporary parents keep private state readable and clean up', () => {
  const outer = fs.mkdtempSync(path.join(os.tmpdir(), 'library-temp-alias-'));
  const target = path.join(outer, 'real');
  const alias = path.join(outer, 'alias');
  const previous = Object.fromEntries(['TMPDIR', 'TMP', 'TEMP'].map((key) => [key, process.env[key]]));
  try {
    fs.mkdirSync(target);
    fs.symlinkSync(target, alias, process.platform === 'win32' ? 'junction' : 'dir');
    Object.assign(process.env, { TMPDIR: alias, TMP: alias, TEMP: alias });
    const directory = probe(`import fs from 'node:fs'; import assert from 'node:assert/strict';
      fs.writeFileSync(process.env.FRAMERIZE_REALM_STATE, '{}');
      assert.equal(fs.readFileSync(process.env.FRAMERIZE_REALM_STATE, 'utf8'), '{}');
      assert.equal(fs.realpathSync(process.env.HOME), process.env.HOME);
      console.log(process.env.HOME);`);
    assert.equal(fs.existsSync(directory), false);
    assert.deepEqual(fs.readdirSync(target), []);
  } finally {
    for (const [key, value] of Object.entries(previous)) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
    fs.rmSync(outer, { recursive: true, force: true });
  }
});

test('network guards reject attempts before connecting, resolving, or listening', () => {
  probe(`import assert from 'node:assert/strict';
    import net from 'node:net'; import tls from 'node:tls';
    import http from 'node:http'; import https from 'node:https';
    import http2 from 'node:http2'; import dgram from 'node:dgram'; import dns from 'node:dns';
    for (const operation of [() => net.connect(-1),
      () => new net.Socket().connect(-1), () => net.createServer().listen(-1),
      () => http.createServer().listen(-1), () => tls.connect(-1),
      () => http.get('invalid URL'), () => https.request('invalid URL'),
      () => http2.connect('invalid URL'), () => dgram.createSocket('invalid type'),
      () => dns.lookup({}, () => {}), () => dns.promises.resolve({}).catch(() => {}),
      () => new dns.Resolver().resolve4({}, () => {}),
      () => fetch('invalid URL').catch(() => {}), () => new WebSocket('invalid URL')]) {
      assert.throws(operation, { code: 'ERR_FIXTURE_OFFLINE' });
    }`);
});

test('Node subprocesses cannot remove the preload or private environment', () => {
  probe(`import assert from 'node:assert/strict'; import cp from 'node:child_process';
    import { promisify } from 'node:util';
    const code = "const assert = require('node:assert/strict');" +
      "assert.equal(process.env.FIXTURE_SECRET, undefined);" +
      "assert.equal(process.env.HOME, process.env.FRAMERIZE_OFFLINE_ROOT);" +
      "assert.throws(() => require('node:net').connect(-1), {code:'ERR_FIXTURE_OFFLINE'});" +
      "const absent = process.env.HOME + '-absent-' + require('node:crypto').randomUUID();" +
      "assert.throws(() => require('node:fs').readFileSync(absent), {code:'ERR_FIXTURE_OFFLINE'});";
    const options = { env: { NODE_OPTIONS: '', FIXTURE_SECRET: 'inert-canary' }, encoding: 'utf8' };
    cp.execFileSync(process.execPath, ['--eval', code], options);
    const sync = cp.spawnSync(process.execPath, ['--eval', code], options);
    assert.equal(sync.status, 0, sync.stderr);
    await promisify(cp.execFile)(process.execPath, ['--eval', code], options);
    await new Promise((resolve, reject) => {
      const child = cp.spawn(process.execPath, ['--eval', code], options);
      child.on('error', reject);
      child.on('close', (status) => status === 0 ? resolve() : reject(new Error('child failed')));
    });
    assert.throws(() => cp.exec('inert-command'), { code: 'ERR_FIXTURE_OFFLINE' });
    assert.throws(() => cp.spawnSync('inert-command'), { code: 'ERR_FIXTURE_OFFLINE' });`);
});

test('existing library fixtures retain their five cases and three Windows skips', () => {
  const source = fs.readFileSync(new URL('../framerize-castle/test/castle.test.js', import.meta.url), 'utf8');
  assert.equal([...source.matchAll(/^test\('/gm)].length, 5);
  assert.equal([...source.matchAll(/skip: !POSIX/g)].length, 3);
  assert.match(source, /const POSIX = process.platform !== 'win32'/);
});

test('fixture encoder subprocess inherits filesystem and network denial', () => {
  probe(`import assert from 'node:assert/strict'; import fs from 'node:fs';
    import path from 'node:path'; import { execFile } from 'node:child_process';
    import { promisify } from 'node:util';
    const script = path.join(process.env.HOME, 'inert-encoder.mjs');
    const body = "import assert from 'node:assert/strict'; import net from 'node:net'; import fs from 'node:fs';" +
      "assert.throws(() => net.createServer().listen(-1), {code:'ERR_FIXTURE_OFFLINE'});" +
      "const absent = process.env.HOME + '-absent-' + crypto.randomUUID();" +
      "assert.throws(() => fs.readFileSync(absent), {code:'ERR_FIXTURE_OFFLINE'});";
    fs.writeFileSync(script, '#!' + process.execPath + '\\n' + body, { mode: 0o700 });
    const command = process.platform === 'win32' ? process.execPath : script;
    await promisify(execFile)(command, process.platform === 'win32' ? [script] : []);
    fs.writeFileSync(script, 'not a Node encoder');
    assert.throws(() => execFile(script), { code: 'ERR_FIXTURE_OFFLINE' });`);
});
