import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createConnection, createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const packageRoot = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));
const preload = fileURLToPath(new URL('./helpers/setup-command-preload.js', import.meta.url));

async function bounded(promise) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error('Launcher fixture timed out')), 5000);
    })]);
  } finally { clearTimeout(timer); }
}

async function unusedPort() {
  const server = createServer();
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const { port } = server.address();
  await new Promise(resolve => server.close(resolve));
  return port;
}

const exists = path => readFile(path).then(() => true, error => {
  if (error.code === 'ENOENT') return false;
  throw error;
});

for (const signal of ['SIGINT', 'SIGTERM']) for (const existing of [false, true]) {
  test(`up rolls back a ${existing ? 'saved' : 'new'} profile after ${signal}`, {
    timeout: 8000, skip: process.platform !== 'linux' && 'Uses a synthetic systemd runner',
  }, async t => {
    const root = await mkdtemp(join(tmpdir(), 'server-up-cancel-'));
    const home = join(root, 'server');
    const settings = join(home, 'server.json');
    const statePath = join(root, 'commands.json');
    const unit = join(root, 'xdg', 'systemd', 'user', 'framerize-server.service');
    const port = await unusedPort();
    const prior = `${JSON.stringify({ version: 1, config: {
      backend: 'http', host: '127.0.0.1', port, upstream: 'http://127.0.0.1:2',
    }, environment: {} })}\n`;
    if (existing) {
      await mkdir(home);
      await writeFile(settings, prior, { mode: 0o600 });
    }
    const child = spawn(process.execPath, ['--import', preload,
      join(packageRoot, 'bin', 'framerize-server.js'), 'up', '--no-lan',
      '--backend', 'http', '--host', '127.0.0.1', '--port', String(port),
      '--upstream', 'http://127.0.0.1:1', '--json'], {
      env: { HOME: root, XDG_CONFIG_HOME: join(root, 'xdg'),
        FRAMERIZE_SERVER_HOME: home, FRAMERIZE_SETUP_FIXTURE_PATH: statePath },
      stdio: ['ignore', 'pipe', 'pipe', 'ipc'],
    });
    let output = '', errors = '';
    child.stdout.setEncoding('utf8').on('data', chunk => { output += chunk; });
    child.stderr.setEncoding('utf8').on('data', chunk => { errors += chunk; });
    const closed = new Promise(resolve => child.once('close', (code, endedBy) => resolve({ code, endedBy })));
    t.after(async () => {
      if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
      await bounded(closed);
      const socket = createConnection({ host: '127.0.0.1', port });
      try {
        const [error] = await bounded(once(socket, 'error'));
        assert.equal(error.code, 'ECONNREFUSED');
      } finally { socket.destroy(); await rm(root, { recursive: true, force: true }); }
    });
    const [message] = await bounded(Promise.race([once(child, 'message'), closed.then(result => {
      throw new Error(`Launcher exited before readiness: ${JSON.stringify(result)} ${errors}`);
    })]));
    assert.deepEqual(message, { event: 'readiness', port });
    assert.equal(child.kill(signal), true);
    const ended = await bounded(closed);
    const state = JSON.parse(await readFile(statePath, 'utf8'));
    const hasUnit = await exists(unit);
    const saved = await readFile(settings, 'utf8').catch(error => {
      if (error.code === 'ENOENT') return undefined;
      throw error;
    });
    t.diagnostic(JSON.stringify({ signal, existing, ended, hasUnit, running: state.running,
      enabled: state.enabled, restored: saved === (existing ? prior : undefined) }));
    assert.deepEqual(ended, { code: 1, endedBy: null });
    assert.equal(saved, existing ? prior : undefined);
    assert.equal(hasUnit, false);
    assert.equal(state.running, false);
    assert.equal(state.enabled, false);
    assert.doesNotMatch(output, /pairing|This machine/);
    assert.match(errors, /cancelled/i);
  });
}
