import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdtemp, rm } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

const packageRoot = process.env.FRAMERIZE_SERVER_PACKAGE_ROOT
  ?? fileURLToPath(new URL('../', import.meta.url));

async function bounded(promise, message, milliseconds = 2000) {
  let timer;
  try {
    return await Promise.race([promise, new Promise((_, reject) => {
      timer = setTimeout(() => reject(new Error(message)), milliseconds);
    })]);
  } finally { clearTimeout(timer); }
}

async function worker() {
  process.channel.ref();
  const { run } = await import(pathToFileURL(join(packageRoot, 'dist/cli.js')));
  const selector = createServer().listen(0, '127.0.0.1');
  await once(selector, 'listening');
  const { port } = selector.address();
  await new Promise(resolve => selector.close(resolve));
  let active = false, rollingBack = false;
  await run(['up', '--backend', 'http', '--upstream', 'http://127.0.0.1:1',
    '--host', '127.0.0.1', '--port', String(port), '--no-lan', '--no-tailscale'], {
    env: { HOME: process.env.HOME, FRAMERIZE_SERVER_HOME: join(process.env.HOME, 'state'),
      XDG_CONFIG_HOME: join(process.env.HOME, 'xdg') },
    setup: { platform: 'linux', uid: 1000,
      run: async (command, args) => {
        assert.ok(['systemctl', 'loginctl'].includes(command));
        if (rollingBack) {
          process.send({ phase: 'rollback', listeners:
            ['SIGINT', 'SIGTERM'].map(signal => process.listenerCount(signal)) });
          await new Promise(resolve => process.once('message', resolve));
        }
        if (command === 'loginctl') return { code: 0, stdout: 'yes\n' };
        if (args[1] === 'restart') active = true;
        if (args[1] === 'is-enabled') return { code: active ? 0 : 1, stdout: '' };
        return { code: 0, stdout: active
          ? 'ActiveState=active\nMainPID=12345\n' : 'ActiveState=inactive\nMainPID=0\n' };
      },
      ready: async (_origin, _token, signal) => {
        process.send({ phase: 'ready' });
        await new Promise((_, reject) => signal.addEventListener('abort', () => {
          rollingBack = true;
          reject(signal.reason);
        }, { once: true }));
      },
    },
  });
}

if (process.argv[2] === 'worker') {
  await worker().catch(error => {
    process.send({ phase: 'error', message: error.message });
    process.disconnect();
    process.exitCode = 1;
  });
} else for (const signal of ['SIGINT', 'SIGTERM']) {
  test(`a second ${signal} can stop pending setup rollback`, {
    skip: process.platform === 'win32' && 'Requires catchable POSIX signals', timeout: 8000,
  }, async t => {
    const root = await mkdtemp(join(tmpdir(), 'server-second-signal-'));
    const env = Object.fromEntries(Object.entries(process.env).filter(([key]) =>
      !key.startsWith('FRAMERIZE_') && !key.startsWith('FRM_')
      && !['NODE_OPTIONS', 'NODE_PATH', 'NODE_USE_ENV_PROXY'].includes(key)));
    const child = spawn(process.execPath, [fileURLToPath(import.meta.url), 'worker'], {
      env: { ...env, HOME: root, FRAMERIZE_SERVER_PACKAGE_ROOT: packageRoot },
      stdio: ['ignore', 'ignore', 'pipe', 'ipc'],
    });
    let stderr = '';
    child.stderr.on('data', chunk => { stderr = (stderr + chunk).slice(-2048); });
    const closed = once(child, 'close');
    t.after(async () => {
      if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL');
      try { await bounded(closed, 'Child cleanup did not finish'); }
      finally { await rm(root, { recursive: true, force: true }); }
    });
    const [ready] = await bounded(once(child, 'message'), `Readiness was not reached: ${stderr}`);
    assert.equal(ready.phase, 'ready', ready.message);
    const rollback = once(child, 'message');
    assert.equal(child.kill(signal), true);
    const [started] = await bounded(Promise.race([rollback, closed.then(([code, ended]) => {
      throw new Error(`Worker exited before rollback: ${code}/${ended}; ${stderr}`);
    })]), 'Rollback was not reached');
    assert.equal(started.phase, 'rollback', started.message);
    t.diagnostic(JSON.stringify({ signal, rollbackListeners: started.listeners }));
    assert.equal(child.kill(signal), true);
    const [code, exitSignal] = await bounded(closed, 'The second signal did not stop rollback');
    assert.equal(code, null);
    assert.equal(exitSignal, signal);
    assert.deepEqual(started.listeners, [0, 0]);
  });
}
