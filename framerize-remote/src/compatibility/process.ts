import { spawn } from 'node:child_process';
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
  /** Runs with inherited stdio and resolves the exit code. */
  readonly run: (command: ClientCommand, env: NodeJS.ProcessEnv) => Promise<number>;
  /** Captures bounded stdout, or null on failure, timeout or a nonzero exit. */
  readonly capture: (command: ClientCommand, env: NodeJS.ProcessEnv) => Promise<string | null>;
}

const CAPTURE_TIMEOUT_MS = 5000;
const CAPTURE_MAX_BYTES = 64 * 1024;

function stopCapture(child: ReturnType<typeof spawn>, platform: NodeJS.Platform, launch: typeof spawn): void {
  if (!child.pid) { child.kill('SIGKILL'); return; }
  if (platform !== 'win32') {
    try { process.kill(-child.pid, 'SIGKILL'); }
    catch { child.kill('SIGKILL'); }
    return;
  }
  try {
    const killer = launch('taskkill', ['/PID', String(child.pid), '/T', '/F'],
      { stdio: 'ignore', windowsHide: true, shell: false });
    killer.once('error', () => { child.kill('SIGKILL'); });
    killer.once('close', code => { if (code !== 0) child.kill('SIGKILL'); });
    killer.unref();
  } catch { child.kill('SIGKILL'); }
}

/** Real process boundary. Tests inject a ProcessRunner instead. */
export function nodeRunner(platform: NodeJS.Platform = process.platform, launch: typeof spawn = spawn): ProcessRunner {
  return {
    run(command, env) {
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
    capture(command, env) {
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
      });
    },
  };
}
