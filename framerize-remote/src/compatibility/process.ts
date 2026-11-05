import { execFile, spawn } from 'node:child_process';
import type { ClientCommand } from './clients.js';

// Windows quoting and shim dispatch adapted from T3 Code packages/shared/src/shell.ts
// escapeWindowsShellArg and resolveSpawnCommand at 5a906583 (MIT).
const WINDOWS_SHELL_META = /([()\][%!^"`<>&|;, *?])/g;

export function escapeWindowsShellArg(arg: string): string {
  const escaped = `"${arg.replace(/(\\*)"/g, '$1$1\\"').replace(/(\\*)$/, '$1$1')}"`;
  return escaped.replace(WINDOWS_SHELL_META, '^$1');
}

/** npm and other shims are .cmd files, which Windows can start only through cmd.exe. */
export function spawnPlan(command: ClientCommand, platform: NodeJS.Platform) {
  if (platform !== 'win32' || !/\.(?:cmd|bat)$/i.test(command.command)) {
    return { file: command.command, args: [...command.args], shell: false };
  }
  return { file: [command.command, ...command.args].map(escapeWindowsShellArg).join(' '), args: [], shell: true };
}

export interface ProcessRunner {
  /** Runs with inherited stdio; cancellation stops the owned command before resolving. */
  readonly run: (command: ClientCommand, env: NodeJS.ProcessEnv, signal?: AbortSignal) => Promise<number>;
  /** Captures bounded stdout, or null on failure, timeout, cancellation or a nonzero exit. */
  readonly capture: (command: ClientCommand, env: NodeJS.ProcessEnv, signal?: AbortSignal) => Promise<string | null>;
}

const CAPTURE_TIMEOUT_MS = 5000;
const CAPTURE_MAX_BYTES = 64 * 1024;
const COMMAND_STOP_GRACE_MS = 500;

// Maintenance keeps its terminal session. Snapshot descendants instead of signaling its shared group.
function collectDescendants(owned: Set<number>): Promise<void> {
  return new Promise((resolve, reject) => {
    execFile('/bin/ps', ['-A', '-o', 'pid=,ppid='],
      { timeout: 1000, killSignal: 'SIGKILL', maxBuffer: 1024 * 1024 }, (error, stdout) => {
        if (error) { reject(error); return; }
        const rows = stdout.split('\n').map(line => line.trim().split(/\s+/).map(Number));
        let before: number;
        do {
          before = owned.size;
          for (const [pid, parent] of rows) {
            if (pid !== undefined && pid > 0 && parent !== undefined && owned.has(parent)) owned.add(pid);
          }
        } while (owned.size !== before);
        resolve();
      });
  });
}

async function stopCommandTree(child: ReturnType<typeof spawn>): Promise<void> {
  if (!child.pid) { child.kill('SIGKILL'); return; }
  const owned = new Set([child.pid]);
  const failures: unknown[] = [];
  const collect = async (): Promise<void> => {
    try { await collectDescendants(owned); } catch (error) { failures.push(error); }
  };
  const send = (signal: NodeJS.Signals): void => {
    for (const pid of [...owned].reverse()) {
      if (pid === child.pid && (child.exitCode !== null || child.signalCode !== null)) continue;
      try { process.kill(pid, signal); }
      catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ESRCH') failures.push(error); }
    }
  };
  await collect();
  send('SIGTERM');
  await new Promise(resolve => setTimeout(resolve, COMMAND_STOP_GRACE_MS));
  await collect();
  send('SIGKILL');
  if (failures.length) throw new AggregateError(failures, 'Could not inspect or stop the client process tree.');
}

function stopCapture(child: ReturnType<typeof spawn>, platform: NodeJS.Platform, launch: typeof spawn,
  stopped?: () => void): void {
  if (!child.pid) { child.kill('SIGKILL'); stopped?.(); return; }
  if (platform !== 'win32') {
    try { process.kill(-child.pid, 'SIGKILL'); }
    catch { child.kill('SIGKILL'); }
    stopped?.();
    return;
  }
  try {
    const killer = launch('taskkill', ['/PID', String(child.pid), '/T', '/F'],
      { stdio: 'ignore', windowsHide: true, shell: false,
        ...(stopped ? { timeout: 1000, killSignal: 'SIGKILL' as const } : {}) });
    killer.once('error', () => { child.kill('SIGKILL'); stopped?.(); });
    killer.once('close', code => { if (code !== 0) child.kill('SIGKILL'); stopped?.(); });
    if (!stopped) killer.unref();
  } catch { child.kill('SIGKILL'); stopped?.(); }
}

function runCancellable(command: ClientCommand, env: NodeJS.ProcessEnv, signal: AbortSignal,
  platform: NodeJS.Platform, launch: typeof spawn): Promise<number> {
  if (signal.aborted) return Promise.resolve(1);
  const plan = spawnPlan(command, platform);
  return new Promise((resolve, reject) => {
    const child = launch(plan.file, plan.args, { env: { ...env, ...command.env }, shell: plan.shell,
      stdio: 'inherit', windowsHide: false, detached: false });
    let treeStopped: Promise<void> | undefined;
    let stopError: unknown;
    let cancelled = false;
    let settled = false;
    const cancel = (): void => {
      if (cancelled || settled) return;
      cancelled = true;
      const stopping = platform === 'win32'
        ? new Promise<void>(resolve => stopCapture(child, platform, launch, resolve))
        : stopCommandTree(child);
      treeStopped = stopping.catch(error => { stopError = error; });
    };
    const finish = (): boolean => {
      if (settled) return false;
      settled = true;
      signal.removeEventListener('abort', cancel);
      return true;
    };
    child.once('error', async error => {
      await treeStopped;
      if (finish()) reject(stopError ?? error);
    });
    child.once('close', async (code, exitSignal) => {
      await treeStopped;
      if (!finish()) return;
      if (stopError) reject(stopError);
      else resolve(cancelled ? 1 : code ?? (exitSignal ? 1 : 0));
    });
    signal.addEventListener('abort', cancel, { once: true });
    if (signal.aborted) cancel();
  });
}

/** Real process boundary. Tests inject a ProcessRunner instead. */
export function nodeRunner(platform: NodeJS.Platform = process.platform, launch: typeof spawn = spawn): ProcessRunner {
  return {
    run(command, env, signal) {
      if (signal) return runCancellable(command, env, signal, platform, launch);
      const plan = spawnPlan(command, platform);
      return new Promise((resolve, reject) => {
        const child = launch(plan.file, plan.args, { env: { ...env, ...command.env }, shell: plan.shell,
          stdio: 'inherit', windowsHide: false });
        // Ctrl+C reaches the child through the terminal; the parent waits for it to exit.
        const forward = (signal: NodeJS.Signals): void => { child.kill(signal); };
        const ignore = (): void => {};
        process.on('SIGINT', ignore);
        process.on('SIGTERM', forward);
        const done = (): void => { process.off('SIGINT', ignore); process.off('SIGTERM', forward); };
        child.once('error', error => { done(); reject(error); });
        child.once('close', (code, signal) => { done(); resolve(code ?? (signal ? 1 : 0)); });
      });
    },
    capture(command, env, signal) {
      if (signal?.aborted) return Promise.resolve(null);
      const plan = spawnPlan(command, platform);
      return new Promise(resolve => {
        let child: ReturnType<typeof spawn>;
        try { child = launch(plan.file, plan.args, { env: { ...env, ...command.env }, shell: plan.shell,
          stdio: ['ignore', 'pipe', 'ignore'], windowsHide: true, detached: platform !== 'win32' }); }
        catch { resolve(null); return; }
        const chunks: Buffer[] = [];
        let total = 0;
        let settled = false;
        const finish = (value: string | null): void => {
          if (settled) return;
          settled = true;
          clearTimeout(timer);
          signal?.removeEventListener('abort', fail);
          resolve(value);
        };
        const fail = (): void => {
          if (settled) return;
          finish(null);
          child.stdout?.destroy();
          stopCapture(child, platform, launch);
          child.unref();
        };
        const timer = setTimeout(fail, CAPTURE_TIMEOUT_MS);
        child.stdout?.on('data', (data: Buffer) => {
          if (settled) return;
          total += data.length;
          if (total > CAPTURE_MAX_BYTES) fail(); else chunks.push(data);
        });
        child.on('error', fail);
        child.stdout?.on('error', fail);
        child.once('close', code => {
          finish(code !== 0 ? null : Buffer.concat(chunks).toString('utf8'));
        });
        signal?.addEventListener('abort', fail, { once: true });
        if (signal?.aborted) fail();
      });
    },
  };
}
