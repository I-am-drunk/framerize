// Service activation and rollback adapted from T3 Code cli/up.ts and
// cloud/bootService.ts at 5a906583fed3fa41dc806f2a19bd9d095537ed3e. MIT, T3 Tools Inc.
import { execFile, spawn } from 'node:child_process';
import { mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { randomUUID } from 'node:crypto';
import { selectServiceManager, serviceHomeOf, type ServicePlan, type ServiceStep } from './service-units.js';

export class ServerSetupError extends Error {}
export function rollbackError(original: unknown, failures: readonly unknown[]): unknown {
  if (failures.length === 0) return original;
  const message = (error: unknown) => error instanceof Error ? error.message : String(error);
  return new ServerSetupError(`${message(original)} Rollback also failed: ${failures.map(message).join('; ')}`,
    { cause: new AggregateError([original, ...failures]) });
}
export type StepRunner = (command: string, args: readonly string[], signal?: AbortSignal) =>
  Promise<{ code: number; stdout: string }>;

async function waitForCommandGroup(group: number): Promise<void> {
  const deadline = Date.now() + 1000;
  for (;;) {
    const remaining = deadline - Date.now();
    if (remaining <= 0) throw new Error('The command process group did not stop.');
    const states = await new Promise<string>((resolve, reject) => {
      execFile('/bin/ps', ['-A', '-o', 'pgid=,stat='], {
        timeout: Math.min(500, remaining), maxBuffer: 1024 * 1024, encoding: 'utf8',
      }, (error, stdout) => error ? reject(error) : resolve(stdout));
    });
    const running = states.split('\n').some(line => {
      const entry = /^\s*(\d+)\s+(\S+)/.exec(line);
      return entry && Number(entry[1]) === group && !/^[ZX]/.test(entry[2]!);
    });
    if (!running) return;
    await new Promise(resolve => setTimeout(resolve, 20));
  }
}

export const runStep: StepRunner = (command, args, signal) => new Promise((resolve, reject) => {
  if (signal?.aborted) { resolve({ code: 1, stdout: '' }); return; }
  const output: Buffer[] = [];
  let stdoutBytes = 0;
  let stderrBytes = 0;
  let stopping: Promise<void> | undefined;
  let forced = false;
  const child = spawn(command, [...args], {
    stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true,
    detached: process.platform !== 'win32',
  });
  const stop = (): void => {
    forced = true;
    if (stopping) return;
    stopping = new Promise<void>((done, rejectStop) => {
      const fail = (cause: unknown): void => rejectStop(new ServerSetupError(
        'Could not stop service command processes. Some may still be running.', { cause }));
      const send = (kind: NodeJS.Signals): boolean => {
        try {
          if (process.platform !== 'win32' && child.pid) return process.kill(-child.pid, kind);
          return child.kill(kind);
        } catch (error) {
          if ((error as NodeJS.ErrnoException).code !== 'ESRCH') {
            fail(error);
          }
          return false;
        }
      };
      if (!send('SIGTERM')) { done(); return; }
      setTimeout(async () => {
        if (send('SIGKILL') && process.platform !== 'win32' && child.pid) {
          try { await waitForCommandGroup(child.pid); }
          catch (error) { fail(error); return; }
        }
        done();
      }, 1000);
    });
    void stopping.catch(error => {
      clearTimeout(timer);
      signal?.removeEventListener('abort', abort);
      child.stdout?.destroy();
      child.stderr?.destroy();
      child.unref();
      reject(error);
    });
  };
  child.stdout.on('data', (chunk: Buffer) => {
    const remaining = Math.max(0, 256 * 1024 - stdoutBytes);
    if (remaining) output.push(chunk.subarray(0, remaining));
    stdoutBytes += chunk.length;
    if (stdoutBytes > 256 * 1024) stop();
  });
  child.stderr.on('data', (chunk: Buffer) => {
    stderrBytes += chunk.length;
    if (stderrBytes > 256 * 1024) stop();
  });
  child.stdout.once('error', stop);
  child.stderr.once('error', stop);
  child.once('error', stop);
  const timer = setTimeout(stop, 120_000).unref();
  const abort = stop;
  signal?.addEventListener('abort', abort, { once: true });
  if (signal?.aborted) abort();
  child.once('exit', (code, exitSignal) => {
    if (code !== 0 || exitSignal) stop();
  });
  child.once('close', code => {
    clearTimeout(timer);
    signal?.removeEventListener('abort', abort);
    void (stopping ?? Promise.resolve()).then(() => {
      resolve({ code: forced || code !== 0 ? 1 : 0, stdout: Buffer.concat(output).toString('utf8') });
    }, reject);
  });
});

export function cancellableRunner(run: StepRunner, signal?: AbortSignal): StepRunner {
  if (!signal) return run;
  return async (command, args) => {
    signal.throwIfAborted();
    const result = await run(command, args, signal);
    signal.throwIfAborted();
    return result;
  };
}

export async function ensureServiceHost(plan: ServicePlan, run: StepRunner = runStep): Promise<void> {
  requiredManager(plan);
  if (plan.platform !== 'linux') return;
  if ((await run('systemctl', ['--user', 'show-environment'])).code !== 0) {
    throw new ServerSetupError('The systemd user manager is unavailable. Use serve in this session.');
  }
  const args = ['show-user', String(plan.uid), '--property=Linger', '--value'];
  const status = await run('loginctl', args);
  if (status.code === 0 && status.stdout.trim() === 'yes') return;
  if (status.code !== 0 || status.stdout.trim() !== 'no') {
    throw new ServerSetupError('Could not read systemd linger status. Use serve in this session.');
  }
  await steps(run, [{ command: 'loginctl', args: ['enable-linger', '--no-ask-password', String(plan.uid)] }]);
  const repaired = await run('loginctl', args);
  if (repaired.code !== 0 || repaired.stdout.trim() !== 'yes') {
    throw new ServerSetupError('Could not enable background startup after logout. Use serve in this session.');
  }
}

async function readOptional(path: string): Promise<string | undefined> {
  try { return await readFile(path, 'utf8'); }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return undefined;
    throw error;
  }
}

export async function writeAtomic(path: string, contents: string): Promise<void> {
  await mkdir(dirname(path), { recursive: true });
  const temporary = `${path}.${randomUUID()}.tmp`;
  try { await writeFile(temporary, contents, { mode: 0o600 }); await rename(temporary, path); }
  finally { await rm(temporary, { force: true }); }
}

async function steps(run: StepRunner, entries: readonly ServiceStep[]): Promise<void> {
  for (const entry of entries) {
    if ((await run(entry.command, entry.args)).code !== 0 && !entry.optional) {
      throw new ServerSetupError(`Service command failed: ${entry.command} ${entry.args.join(' ')}`);
    }
  }
}

export async function serviceStatus(plan: ServicePlan, run: StepRunner = runStep) {
  const manager = selectServiceManager(plan);
  if (!manager) return { supported: false, installed: false, running: false, enabled: false, current: false, otherHome: false };
  const contents = await readOptional(manager.path);
  const installed = contents !== undefined;
  const installedHome = contents && serviceHomeOf(contents);
  const otherHome = installed && (!installedHome || resolve(installedHome) !== resolve(plan.home));
  const state = installed ? await run(manager.active.command, manager.active.args) : undefined;
  const loaded = state?.code === 0;
  const output = state?.stdout ?? '';
  const activeState = /^ActiveState=(\S+)$/m.exec(output)?.[1];
  const pid = Number(plan.platform === 'linux' ? /^MainPID=(\d+)$/m.exec(output)?.[1]
    : /^\s*pid = (\d+)\s*$/m.exec(output)?.[1]);
  const active = plan.platform === 'linux'
    ? /^ActiveState=active$/m.test(output) && /^SubState=running$/m.test(output)
    : /^\s*state = running\s*$/m.test(output);
  const running = loaded && active && Number.isSafeInteger(pid) && pid > 0;
  const enabled = !manager.enabled || (await run(manager.enabled.command, manager.enabled.args)).code === 0;
  return { supported: true, installed, running, loaded, activeState, pid: running ? pid : undefined,
    enabled, current: contents === manager.contents && enabled, otherHome,
    unitPath: manager.path, logPath: join(plan.home, 'server.log') };
}

function requiredManager(plan: ServicePlan) {
  const manager = selectServiceManager(plan);
  if (!manager) throw new ServerSetupError('Background service setup supports Linux and macOS. Use serve on this system.');
  return manager;
}

export async function reconcileService(plan: ServicePlan, settings: string,
  ready: () => Promise<number | void>, run: StepRunner = runStep,
  beforeStart: () => Promise<void> = async () => {}, signal?: AbortSignal):
  Promise<'installed' | 'updated' | 'unchanged'> {
  const manager = requiredManager(plan);
  const activeRun = cancellableRunner(run, signal);
  const before = await serviceStatus(plan, activeRun);
  if (before.otherHome) throw new ServerSetupError('The Server service belongs to another FRAMERIZE_SERVER_HOME.');
  const settingsPath = join(plan.home, 'server.json');
  const [oldSettings, oldUnit] = await Promise.all([readOptional(settingsPath), readOptional(manager.path)]);
  if (before.running && before.current && oldSettings === settings) {
    await verifyReady(await ready());
    return 'unchanged';
  }
  try {
    if (before.installed && (plan.platform === 'linux' || before.loaded)) await steps(activeRun, manager.stop);
    await beforeStart();
    signal?.throwIfAborted();
    await writeAtomic(settingsPath, settings);
    signal?.throwIfAborted();
    await writeAtomic(manager.path, manager.contents);
    await steps(activeRun, manager.activate);
    await verifyReady(await ready());
  } catch (error) {
    try { await restoreService(); }
    catch (failure) { throw rollbackError(error, [failure]); }
    throw error;
  }
  return before.installed ? 'updated' : 'installed';

  async function verifyReady(pid: number | void): Promise<void> {
    const after = await serviceStatus(plan, activeRun);
    if (!after.running || (pid !== undefined && after.pid !== pid)) {
      throw new ServerSetupError('The Server service is not running or another process answered readiness. Check server.log.');
    }
  }

  async function restoreService(): Promise<void> {
    await steps(run, manager.remove.map(step => ({ ...step, optional: true })));
    if (oldSettings === undefined) await rm(settingsPath, { force: true });
    else await writeAtomic(settingsPath, oldSettings);
    if (oldUnit === undefined) await rm(manager.path, { force: true });
    else await writeAtomic(manager.path, oldUnit);
    await steps(run, manager.reload);
    if (oldUnit !== undefined && before.enabled) await steps(run, manager.enable);
    const restore = before.running || (plan.platform === 'linux'
      ? before.enabled && before.activeState !== undefined && before.activeState !== 'inactive' : before.loaded);
    if (oldUnit !== undefined && restore) await steps(run, manager.activate);
  }
}

export async function controlService(plan: ServicePlan, action: 'start' | 'stop' | 'restart' | 'uninstall',
  run: StepRunner = runStep): Promise<'done' | 'missing'> {
  const manager = requiredManager(plan);
  const status = await serviceStatus(plan, run);
  if (!status.installed) return 'missing';
  if (status.otherHome) throw new ServerSetupError('The Server service belongs to another FRAMERIZE_SERVER_HOME.');
  if (action === 'uninstall') {
    if (plan.platform !== 'darwin' || status.loaded) await steps(run, manager.remove);
    await rm(manager.path, { force: true });
    await steps(run, manager.reload);
  } else if (action !== 'stop' || status.loaded) {
    const needsRestart = plan.platform === 'darwin' ? status.loaded : status.running && action === 'restart';
    await steps(run, action === 'stop' ? manager.stop
      : needsRestart ? manager.restart : manager.activate);
  }
  return 'done';
}
