import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

const packageRoot = process.env.FRAMERIZE_REMOTE_PACKAGE_ROOT
  ?? fileURLToPath(new URL('..', import.meta.url));
const launcher = join(packageRoot, 'bin/frm.js');
const downloader = `#!${process.execPath}
const fs = require('node:fs');
fs.writeFileSync(process.env.FIXTURE_CURL_LOG, JSON.stringify(process.argv.slice(2)));
process.stdout.write(process.env.FIXTURE_SCRIPT, () => {
  process.exitCode = Number(process.env.FIXTURE_CURL_EXIT);
});
`;

const networkGuard = `import net from 'node:net';
import fs from 'node:fs/promises';
import { syncBuiltinESMExports } from 'node:module';
net.Socket.prototype.connect = () => { throw new Error('Fixture forbids network'); };
globalThis.fetch = () => { throw new Error('Fixture forbids network'); };
const originalRead = fs.readFile;
fs.readFile = (path, ...args) => String(path).includes('managed-settings.json')
  ? Promise.reject(Object.assign(new Error('Private fixture'), { code: 'ENOENT' }))
  : originalRead(path, ...args);
syncBuiltinESMExports();
`;
const installer = 'printf ran > "$FIXTURE_EXECUTED"\n'
  + 'printf "%s" "$FIXTURE_CLIENT_SOURCE" > "$FIXTURE_CLIENT_PATH"\n'
  + 'chmod 700 "$FIXTURE_CLIENT_PATH"\nexit "$FIXTURE_INSTALL_EXIT"\n';

async function optionalText(path) {
  try { return await readFile(path, 'utf8'); }
  catch (error) { if (error.code === 'ENOENT') return null; throw error; }
}

async function invoke(args, env, cwd) {
  let child;
  const result = new Promise(resolve => {
    child = execFile(process.execPath, [launcher, ...args], { env, cwd, detached: true,
      timeout: 5000, killSignal: 'SIGKILL', maxBuffer: 65536 }, (error, stdout, stderr) => {
      resolve({ code: error?.code ?? 0, signal: error?.signal ?? null, stdout, stderr });
    });
  });
  const closed = new Promise(resolve => child.once('close', resolve));
  try { return await result; }
  finally {
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
    await closed;
  }
}

const modes = [
  { name: 'declined', consent: false, download: 0, install: 0, code: 3, executes: false, launches: false },
  { name: 'partial download', consent: true, download: 18, install: 0, code: 1, executes: false, launches: false },
  { name: 'successful install', consent: true, download: 0, install: 0, code: 0, executes: true, launches: true },
  { name: 'installer failure', consent: true, download: 0, install: 23, code: 1, executes: true, launches: false },
];

for (const client of ['claude', 'codex']) for (const mode of modes) {
  test(`client CLI ${client}: ${mode.name}`, {
    timeout: 10000, skip: process.platform === 'win32' ? 'POSIX installer fixture' : false,
  }, async () => {
    const root = await mkdtemp(join(tmpdir(), 'frm-installer-cli-'));
    const bin = join(root, 'bin');
    const temporary = join(root, 'temporary with spaces');
    const profile = join(root, 'profile');
    try {
      await mkdir(bin);
      await mkdir(temporary);
      for (const name of ['bash', 'sh', 'chmod', 'mktemp', 'rm']) {
        const path = ['/bin', '/usr/bin'].map(directory => join(directory, name)).find(existsSync);
        assert.ok(path, `Fixture requires ${name}`);
        await symlink(path, join(bin, name));
      }
      await writeFile(join(bin, 'curl'), downloader, { mode: 0o700 });
      const guard = join(root, 'guard.mjs');
      await writeFile(guard, networkGuard);
      const executed = join(root, 'executed');
      const launched = join(root, 'launched');
      const curlLog = join(root, 'curl.json');
      const clientSource = `#!${process.execPath}\n`
        + `require('node:fs').writeFileSync(process.env.FIXTURE_LAUNCHED, JSON.stringify(process.argv.slice(2)));\n`;
      const env = { HOME: root, USERPROFILE: root, PATH: bin, TMPDIR: temporary,
        FRM_HOME: profile, FRM_REMOTE: 'http://127.0.0.1:1', FRM_PORT: '8788',
        CLAUDE_CONFIG_DIR: join(root, 'claude'), NODE_DISABLE_COMPILE_CACHE: '1',
        NODE_OPTIONS: `--import=${pathToFileURL(guard).href}`,
        FIXTURE_CURL_LOG: curlLog, FIXTURE_CURL_EXIT: String(mode.download),
        FIXTURE_SCRIPT: installer, FIXTURE_EXECUTED: executed, FIXTURE_LAUNCHED: launched,
        FIXTURE_CLIENT_SOURCE: clientSource, FIXTURE_CLIENT_PATH: join(bin, client),
        FIXTURE_INSTALL_EXIT: String(mode.install) };
      const result = await invoke(['client', client, '--no-input',
        ...(mode.consent ? ['--install'] : []), '--', 'synthetic-install-probe'], env, root);
      assert.equal(result.code, mode.code, result.stderr);
      assert.equal(result.signal, null, result.stderr);
      assert.equal(await optionalText(executed), mode.executes ? 'ran' : null);
      const launch = await optionalText(launched);
      if (mode.launches) assert.ok(JSON.parse(launch).includes('synthetic-install-probe'));
      else assert.equal(launch, null);
      assert.match(result.stdout, /Official installer: Download https:\/\//);
      const download = await optionalText(curlLog);
      if (mode.consent) {
        const url = client === 'claude' ? 'https://claude.ai/install.sh' : 'https://chatgpt.com/codex/install.sh';
        assert.deepEqual(JSON.parse(download), ['-fsSL', url]);
      } else assert.equal(download, null);
      if (mode.download || mode.install) {
        assert.match(result.stderr, new RegExp(`installer exited with code ${mode.download || mode.install}`));
      }
      assert.deepEqual(await readdir(temporary), []);
      await assert.rejects(readdir(profile), { code: 'ENOENT' });
    } finally { await rm(root, { recursive: true, force: true }); }
  });
}
