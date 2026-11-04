import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { join } from 'node:path';
import { readFileSync, writeFileSync } from 'node:fs';
import { upstreamToken } from './transport-scenario.js';

export const serverToken = 'synthetic-server-credential';

async function bounded<Value>(promise: Promise<Value>, milliseconds = 5000): Promise<Value> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try { return await Promise.race([promise, new Promise<never>((resolveUnused, reject) => {
    timer = setTimeout(() => reject(new Error('Fixture process timed out')), milliseconds);
  })]); } finally { clearTimeout(timer); }
}

export async function startInstalledServer(consumer: string, env: NodeJS.ProcessEnv, upstream: string,
  nativeExecutable?: string) {
  const accounts = join(consumer, 'accounts.json');
  writeFileSync(accounts, JSON.stringify([{ id: 'synthetic', enabled: true }]));
  return startInstalledCli(consumer, { ...env, FIXTURE_UPSTREAM_TOKEN: upstreamToken, FIXTURE_SERVER_TOKEN: serverToken }, [
    ...(nativeExecutable ? [] : [join(consumer, 'node_modules/framerize-server/bin/framerize-server.js')]),
    'serve', '--port', '0', '--host', '127.0.0.1', '--upstream', upstream,
    '--authorization-env', 'FIXTURE_UPSTREAM_TOKEN', '--server-token-env', 'FIXTURE_SERVER_TOKEN',
    '--accounts-file', accounts], 'server', nativeExecutable);
}

export async function startInstalledAccountServer(consumer: string, env: NodeJS.ProcessEnv, origin: string,
  nativeExecutable?: string) {
  const config = accountServerFixture(consumer, env, origin);
  return startInstalledCli(consumer, config.env, [
    ...(nativeExecutable ? [] : [join(consumer, 'node_modules/framerize-server/bin/framerize-server.js')]),
    'serve', ...config.args,
  ], 'server', nativeExecutable);
}

export function accountServerFixture(consumer: string, env: NodeJS.ProcessEnv, origin: string) {
  const accounts = join(consumer, 'accounts.json');
  writeFileSync(accounts, JSON.stringify([
    { id: 'unused', projectId: 'unused-project', castleDeviceId: 'synthetic-unused-device',
      session: { env: 'UNSET_ACCOUNT_SESSION' } },
    { id: 'selected', projectId: 'selected-project', castleDeviceId: 'synthetic-selected-device',
      session: { env: 'FIXTURE_ACCOUNT_SESSION' } },
  ]));
  return { env: { ...env, FIXTURE_ACCOUNT_SESSION: 'session=synthetic-account',
    FIXTURE_SERVER_TOKEN: serverToken }, args: [
    '--backend', 'framer', '--account-id', 'selected', '--accounts-file', accounts,
    '--framer-origin', origin, '--port', '0', '--server-token-env', 'FIXTURE_SERVER_TOKEN',
  ] };
}

async function startInstalledCli(consumer: string, env: NodeJS.ProcessEnv, args: string[], kind: 'server' | 'remote',
  executable = process.execPath) {
  const child = spawn(executable, args, { cwd: consumer, stdio: ['ignore', 'pipe', 'pipe'], env });
  let output = '';
  let pending = '';
  const exited = new Promise<{ code: number | null; signal: NodeJS.Signals | null }>((resolve, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => resolve({ code, signal }));
  });
  exited.catch(() => {});
  const listening = new Promise<number>((resolve, reject) => {
    child.once('error', reject);
    child.once('exit', () => reject(new Error('Installed server exited before listening')));
    child[kind === 'server' ? 'stdout' : 'stderr'].on('data', (chunk: Buffer) => {
      output += chunk.toString();
      if (output.length > 65536) { child.kill('SIGKILL'); reject(new Error('Excess fixture output')); }
    });
    child[kind === 'server' ? 'stderr' : 'stdout'].on('data', (chunk: Buffer) => {
      output += chunk.toString();
      pending += chunk.toString();
      if (output.length > 65536) { child.kill('SIGKILL'); reject(new Error('Excess fixture output')); }
      while (pending.includes('\n')) {
        const end = pending.indexOf('\n');
        const line = pending.slice(0, end);
        pending = pending.slice(end + 1);
        try {
          let event = JSON.parse(line) as { event?: unknown; port?: unknown; listening?: { port: unknown } };
          if (kind === 'remote' && event.listening) event = { event: 'server.listening', port: event.listening.port };
          if (event.event === 'server.listening') {
            if (typeof event.port !== 'number' || !Number.isInteger(event.port) || event.port < 1 || event.port > 65535) {
              reject(new Error('Invalid installed server address'));
            } else resolve(event.port);
          }
        } catch { reject(new Error('Invalid installed server log')); }
      }
    });
  });
  let port: number;
  try { port = await bounded(listening); }
  catch (error) { child.kill('SIGKILL'); await exited.catch(() => {}); throw error; }
  let stopped: Promise<void> | undefined;
  return { url: `http://127.0.0.1:${port}`, output: () => output, close() {
    stopped ??= (async () => {
      child.kill('SIGTERM');
      let result;
      try { result = await bounded(exited); }
      catch (error) { child.kill('SIGKILL'); await exited; throw error; }
      if (process.platform !== 'win32') assert.deepEqual(result, { code: 0, signal: null });
      else assert.ok(result.code === 0 || result.signal === 'SIGTERM');
      assert.doesNotMatch(output, /synthetic-(?:local|server|upstream)-credential|server\.failed/);
    })();
    return stopped;
  } };
}

export const localToken = 'synthetic-local-credential';

export async function startInstalledRemote(consumer: string, env: NodeJS.ProcessEnv, remote: string,
  options: { saved?: boolean; cwd?: string; serveArgs?: string[] } = {}) {
  const home = join(consumer, 'remote-home');
  const config = join(home, 'framerize-remote.json');
  const launcher = join(consumer, 'node_modules/framerize-remote/bin/framerize-remote.js');
  const args = options.saved ? ['serve', ...(options.serveArgs ?? [])] : ['setup', '--remote', remote,
    '--server-token-env', 'FIXTURE_SERVER_TOKEN', '--local-token-env', 'FIXTURE_LOCAL_TOKEN',
    '--port', '0', '--start'];
  const handle = await startInstalledCli(options.cwd ?? consumer,
    { ...env, FRAMERIZE_REMOTE_HOME: home, FIXTURE_SERVER_TOKEN: serverToken, FIXTURE_LOCAL_TOKEN: localToken },
    [launcher, ...args, '--json'], 'remote');
  try {
    assert.doesNotMatch(readFileSync(config, 'utf8'), /synthetic-(?:local|server)-credential/);
    return handle;
  } catch (error) { await handle.close(); throw error; }
}
