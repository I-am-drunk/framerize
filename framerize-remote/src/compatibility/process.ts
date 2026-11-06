import { spawn } from 'node:child_process';
import { closeSync, openSync } from 'node:fs';
import type { Readable, Writable } from 'node:stream';
import type { ClientCommand } from './clients.js';
import { ClientCleanupError, RemoteDiagnosticError } from '../diagnostics.js';

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
const COMMAND_START_TIMEOUT_MS = 5000;

// Shell job control creates a foreground group without creating a new terminal session.
const TERMINAL_COMMAND = `parent=$1
shift
(
  set +m
  trap 'kill -INT "$parent"' INT
  trap 'kill -TERM "$parent"' TERM
  IFS= read -r decision <&4 || exit 1
  [ "$decision" = run ] || exit 1
  exec 5<&0
  "$@" <&5 3>&- 4>&- 5<&- &
  task=$!
  while :; do
    wait "$task"
    result=$?
    kill -0 "$task" 2>/dev/null || exit "$result"
  done
) &
printf '%s\\n' "$!" >&3
fg %1 >/dev/null 2>/dev/null`;

function hasControllingTerminal(): boolean {
  try { const fd = openSync('/dev/tty', 'r'); closeSync(fd); return true; }
  catch { return false; }
}

async function stopCommandGroup(group: number): Promise<void> {
  const failures: unknown[] = [];
  const send = (signal: NodeJS.Signals): void => {
    try { process.kill(-group, signal); }
    catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ESRCH') failures.push(error); }
  };
  send('SIGTERM');
  await new Promise(resolve => setTimeout(resolve, COMMAND_STOP_GRACE_MS));
  send('SIGKILL');
  if (failures.length) throw new ClientCleanupError(failures);
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
  const terminal = platform !== 'win32' && hasControllingTerminal();
  return new Promise((resolve, reject) => {
    const child = terminal
      ? launch('/bin/sh', ['-m', '-c', TERMINAL_COMMAND, 'frm-maintenance', String(process.pid), plan.file, ...plan.args],
        { env: { ...env, ...command.env }, stdio: ['inherit', 'inherit', 'inherit', 'pipe', 'pipe'], shell: false })
      : launch(plan.file, plan.args, { env: { ...env, ...command.env }, shell: plan.shell,
        stdio: 'inherit', windowsHide: false, detached: platform !== 'win32' });
    const groupChannel = terminal ? child.stdio[3] as Readable : undefined;
    const control = terminal ? child.stdio[4] as Writable : undefined;
    let group = terminal || platform === 'win32' ? undefined : child.pid;
    let treeStopped: Promise<void> | undefined;
    let stopError: unknown;
    let cancelled = false;
    let settled = false;
    let decided = false;
    let startupTimer: ReturnType<typeof setTimeout> | undefined;
    let stopTimer: ReturnType<typeof setTimeout> | undefined;
    const stopUnstarted = (): void => {
      if (group !== undefined || settled) return;
      groupChannel?.destroy();
      control?.destroy();
      child.kill('SIGKILL');
    };
    const decide = (run: boolean): void => {
      if (!control || decided) return;
      decided = true;
      control.end(run ? 'run\n' : 'stop\n');
    };
    const stopGroup = (): void => {
      if (treeStopped || group === undefined) return;
      treeStopped = stopCommandGroup(group).catch(error => { stopError = error; });
    };
    const cancel = (): void => {
      if (cancelled || settled) return;
      cancelled = true;
      decide(false);
      if (platform === 'win32') {
        treeStopped = new Promise<void>(resolve => stopCapture(child, platform, launch, resolve));
      } else if (terminal && group === undefined) {
        stopTimer = setTimeout(stopUnstarted, COMMAND_STOP_GRACE_MS);
      } else stopGroup();
    };
    let groupText = '';
    groupChannel?.on('data', (data: Buffer) => {
      groupText += data.toString('utf8');
      if (groupText.length <= 32 && !groupText.includes('\n')) return;
      const parsed = Number(groupText.trim());
      if (!/^[1-9]\d*\n$/.test(groupText) || !Number.isSafeInteger(parsed)
        || parsed <= 1 || parsed === process.pid || parsed === child.pid) {
        stopError = new RemoteDiagnosticError('client_start_failed', 'Could not identify the client process group.');
        groupChannel?.destroy();
        cancel();
        return;
      }
      group = parsed;
      clearTimeout(startupTimer);
      clearTimeout(stopTimer);
      groupChannel?.destroy();
      if (cancelled || signal.aborted) { cancel(); stopGroup(); }
      else decide(true);
    });
    const failGroup = (error: Error): void => {
      stopError = new RemoteDiagnosticError('client_start_failed', error.message);
      cancel();
    };
    groupChannel?.once('error', failGroup);
    control?.once('error', failGroup);
    groupChannel?.once('end', () => {
      if (group === undefined && !cancelled) failGroup(new Error('The client group did not report its identity.'));
    });
    const finish = (): boolean => {
      if (settled) return false;
      settled = true;
      clearTimeout(startupTimer);
      clearTimeout(stopTimer);
      groupChannel?.destroy();
      control?.destroy();
      signal.removeEventListener('abort', cancel);
      return true;
    };
    child.once('error', async error => {
      await treeStopped;
      if (finish()) reject(stopError ?? new RemoteDiagnosticError('client_start_failed', error.message));
    });
    child.once('close', async (code, exitSignal) => {
      if (code !== 0 || exitSignal) stopGroup();
      await treeStopped;
      if (!finish()) return;
      if (stopError) reject(stopError);
      else resolve(cancelled ? 1 : code ?? (exitSignal ? 1 : 0));
    });
    if (terminal) startupTimer = setTimeout(() => {
      failGroup(new Error('The client process group did not start in time.'));
    }, COMMAND_START_TIMEOUT_MS);
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
      const terminalSignals = platform === 'win32' || hasControllingTerminal();
      return new Promise((resolve, reject) => {
        const child = launch(plan.file, plan.args, { env: { ...env, ...command.env }, shell: plan.shell,
          stdio: 'inherit', windowsHide: false });
        // Terminal SIGINT already reaches the foreground child; other callers must forward it.
        const forward = (signal: NodeJS.Signals): void => { child.kill(signal); };
        const interrupt = (): void => { if (!terminalSignals) child.kill('SIGINT'); };
        process.on('SIGINT', interrupt);
        process.on('SIGTERM', forward);
        const done = (): void => { process.off('SIGINT', interrupt); process.off('SIGTERM', forward); };
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
          if (code !== 0) fail();
          else finish(Buffer.concat(chunks).toString('utf8'));
        });
        signal?.addEventListener('abort', fail, { once: true });
        if (signal?.aborted) fail();
      });
    },
  };
}
