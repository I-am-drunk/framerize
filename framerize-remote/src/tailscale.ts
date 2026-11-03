import { spawn } from 'node:child_process';
import { isIPv4 } from 'node:net';

const labels = {
  running: 'The local Tailscale daemon is running with a tailnet IPv4 address.',
  not_running: 'The local Tailscale daemon is not running.',
  needs_login: 'The local Tailscale installation needs login or approval.',
  no_address: 'The local Tailscale daemon reports no tailnet IPv4 address.',
  missing: 'The Tailscale executable was not found.',
  permission_denied: 'Permission to inspect local Tailscale status was denied.',
  timeout: 'The local Tailscale status check timed out.',
  cancelled: 'The local Tailscale status check was cancelled.',
  output_limit: 'The local Tailscale status exceeded its output limit.',
  invalid_response: 'The local Tailscale status response was invalid.',
  failed: 'The local Tailscale status check failed.',
} as const;

export type TailscaleState = keyof typeof labels;
export interface TailscaleOptions {
  readonly signal?: AbortSignal;
  readonly env?: NodeJS.ProcessEnv;
  readonly platform?: NodeJS.Platform;
  readonly launch?: typeof spawn;
}
export const TAILSCALE_STATUS_TIMEOUT_MS = 1500;
const MAX_OUTPUT_BYTES = 256 * 1024;

function report(state: TailscaleState) {
  return { state, message: labels[state], scope: 'local_only' as const };
}

function stderrState(text: string): TailscaleState {
  if (/not logged in|logged out|needs? login/i.test(text)) return 'needs_login';
  if (/permission denied|access denied|must be root|operation not permitted/i.test(text)) return 'permission_denied';
  return 'failed';
}

function processError(error: unknown): TailscaleState {
  const code = (error as NodeJS.ErrnoException | undefined)?.code;
  return code === 'ENOENT' ? 'missing' : code === 'EACCES' || code === 'EPERM' ? 'permission_denied' : 'failed';
}

function parseStatus(text: string): TailscaleState {
  try {
    const status = JSON.parse(text) as { BackendState?: unknown; Self?: { TailscaleIPs?: unknown } };
    if (status.BackendState === 'NeedsLogin' || status.BackendState === 'NeedsMachineAuth') return 'needs_login';
    if (['Stopped', 'Starting', 'NoState', 'InUseOtherUser'].includes(String(status.BackendState))) return 'not_running';
    if (status.BackendState !== 'Running' || !Array.isArray(status.Self?.TailscaleIPs)) return 'invalid_response';
    return status.Self.TailscaleIPs.some((address: unknown) => {
      if (typeof address !== 'string' || !isIPv4(address)) return false;
      const [first, second] = address.split('.').map(Number);
      return first === 100 && second! >= 64 && second! <= 127;
    }) ? 'running' : 'no_address';
  } catch { return 'invalid_response'; }
}

export async function readTailscaleStatus(options: TailscaleOptions = {}) {
  if (options.signal?.aborted) return report('cancelled');
  const env: NodeJS.ProcessEnv = {};
  for (const [name, value] of Object.entries(options.env ?? process.env)) {
    if (/^(PATH|SYSTEMROOT|WINDIR|LANG|LC_ALL)$/i.test(name)) env[name] = value;
  }
  const command = (options.platform ?? process.platform) === 'win32' ? 'tailscale.exe' : 'tailscale';
  let child: ReturnType<typeof spawn>;
  try { child = (options.launch ?? spawn)(command, ['status', '--json'], {
    env, shell: false, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
  }); }
  catch (error) { return report(processError(error)); }
  return new Promise<ReturnType<typeof report>>(resolve => {
    let failure: TailscaleState | undefined;
    let total = 0;
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    const stop = (state: TailscaleState): void => {
      if (failure) return;
      failure = state;
      child.kill('SIGKILL');
      child.stdout?.destroy();
      child.stderr?.destroy();
    };
    const abort = (): void => stop('cancelled');
    const timer = setTimeout(() => stop('timeout'), TAILSCALE_STATUS_TIMEOUT_MS);
    const collect = (chunks: Buffer[], data: Buffer): void => {
      if (failure) return;
      total += data.length;
      if (total > MAX_OUTPUT_BYTES) stop('output_limit');
      else chunks.push(data);
    };
    child.stdout?.on('data', (data: Buffer) => collect(stdout, data));
    child.stderr?.on('data', (data: Buffer) => collect(stderr, data));
    child.stdout?.on('error', () => stop('failed'));
    child.stderr?.on('error', () => stop('failed'));
    child.once('error', (error) => { failure ??= processError(error); });
    child.once('close', (code) => {
      clearTimeout(timer);
      options.signal?.removeEventListener('abort', abort);
      resolve(report(failure ?? (code === 0 ? parseStatus(Buffer.concat(stdout).toString('utf8'))
        : stderrState(Buffer.concat(stderr).toString('utf8')))));
    });
    options.signal?.addEventListener('abort', abort, { once: true });
    if (options.signal?.aborted) abort();
  });
}
