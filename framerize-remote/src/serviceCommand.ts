import { execFile, spawn } from 'node:child_process';
import { ServiceCommandCleanupError } from './diagnostics.js';

/** Wait for every owned command before propagating cancellation or a cleanup error. */
export async function waitForServiceCommands<T extends readonly unknown[] | []>(commands: T):
  Promise<{ -readonly [P in keyof T]: Awaited<T[P]> }> {
  const results = await Promise.allSettled(commands);
  const cleanup = results.find(result => result.status === 'rejected'
    && result.reason instanceof ServiceCommandCleanupError);
  if (cleanup?.status === 'rejected') throw cleanup.reason;
  return Promise.all(commands);
}

/** Ordinary probe failures mean unavailable; cleanup failures require explicit attention. */
export function unavailableCommand(error: unknown): undefined {
  if (error instanceof ServiceCommandCleanupError) throw error;
  return undefined;
}

async function waitForGroup(group: number): Promise<void> {
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

/** Bounded noninteractive commands; failed commands stop their owned POSIX group. */
export function executeServiceCommand(command: string, args: readonly string[], timeout: number,
  maxBuffer: number, signal?: AbortSignal): Promise<{ code: number; stdout: string }> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) { resolve({ code: 1, stdout: '' }); return; }
    const output: Buffer[] = [];
    let stdoutBytes = 0, stderrBytes = 0;
    let stopping: Promise<void> | undefined;
    let interrupted = false;
    const child = spawn(command, [...args], {
      stdio: ['ignore', 'pipe', 'pipe'], windowsHide: true, detached: process.platform !== 'win32',
    });
    const stop = (): void => {
      if (stopping) return;
      stopping = new Promise<void>((done, rejectStop) => {
        const fail = (cause: unknown): void => rejectStop(new ServiceCommandCleanupError(cause));
        const send = (kind: NodeJS.Signals): boolean => {
          try {
            if (process.platform !== 'win32' && child.pid) return process.kill(-child.pid, kind);
            return child.kill(kind);
          } catch (error) {
            if ((error as NodeJS.ErrnoException).code !== 'ESRCH') fail(error);
            return false;
          }
        };
        if (!send('SIGTERM')) { done(); return; }
        setTimeout(async () => {
          if (send('SIGKILL') && process.platform !== 'win32' && child.pid) {
            try { await waitForGroup(child.pid); }
            catch (error) { fail(error); return; }
          }
          done();
        }, 1000);
      });
      void stopping.catch(error => {
        clearTimeout(timer);
        signal?.removeEventListener('abort', abort);
        child.stdout.destroy();
        child.stderr.destroy();
        child.unref();
        reject(error);
      });
    };
    const abort = (): void => { interrupted = true; stop(); };
    child.stdout.on('data', (chunk: Buffer) => {
      const remaining = Math.max(0, maxBuffer - stdoutBytes);
      if (remaining) output.push(chunk.subarray(0, remaining));
      stdoutBytes += chunk.length;
      if (stdoutBytes > maxBuffer) abort();
    });
    child.stderr.on('data', (chunk: Buffer) => {
      stderrBytes += chunk.length;
      if (stderrBytes > maxBuffer) abort();
    });
    child.stdout.once('error', abort);
    child.stderr.once('error', abort);
    child.once('error', abort);
    const timer = setTimeout(abort, timeout).unref();
    signal?.addEventListener('abort', abort, { once: true });
    if (signal?.aborted) abort();
    child.once('exit', (code, exitSignal) => {
      if (code !== 0 || exitSignal) stop();
    });
    child.once('close', code => {
      clearTimeout(timer);
      signal?.removeEventListener('abort', abort);
      void (stopping ?? Promise.resolve()).then(() => {
        resolve({ code: interrupted ? 1 : code ?? 1, stdout: Buffer.concat(output).toString('utf8') });
      }, reject);
    });
  });
}
