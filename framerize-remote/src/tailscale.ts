import { spawn } from 'node:child_process';
import { isIPv4 } from 'node:net';
import { ServiceCommandCleanupError } from './diagnostics.js';
import { waitForGroup } from './serviceCommand.js';

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
    return parseTailscaleStatus(text).tailnetIpv4Addresses.length > 0 ? 'running' : 'no_address';
  } catch { return 'invalid_response'; }
}

export async function readTailscaleStatus(options: TailscaleOptions = {}) {
  if (options.signal?.aborted) return report('cancelled');
  const env: NodeJS.ProcessEnv = {};
  for (const [name, value] of Object.entries(options.env ?? process.env)) {
    if (/^(PATH|SYSTEMROOT|WINDIR|LANG|LC_ALL)$/i.test(name)) env[name] = value;
  }
  const platform = options.platform ?? process.platform;
  const command = platform === 'win32' ? 'tailscale.exe' : 'tailscale';
  let child: ReturnType<typeof spawn>;
  try { child = (options.launch ?? spawn)(command, ['status', '--json'], {
    env, shell: false, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
    detached: platform !== 'win32',
  }); }
  catch (error) { return report(processError(error)); }
  return new Promise<ReturnType<typeof report>>((resolve, reject) => {
    let failure: TailscaleState | undefined;
    let stopping: Promise<void> | undefined;
    let total = 0;
    const stdout: Buffer[] = [];
    const stderr: Buffer[] = [];
    const stop = (state: TailscaleState): void => {
      if (failure) return;
      failure = state;
      stopping = Promise.resolve().then(async () => {
        try {
          if (platform !== 'win32' && child.pid) {
            try { process.kill(-child.pid, 'SIGKILL'); }
            catch (error) {
              const code = (error as NodeJS.ErrnoException).code;
              if (code === 'ESRCH') return;
              if (code !== 'EPERM') throw error;
              try { await waitForGroup(child.pid); return; } catch { throw error; }
            }
            await waitForGroup(child.pid);
          } else if (!child.kill('SIGKILL') && child.pid
            && child.exitCode === null && child.signalCode === null) {
            throw new Error('The Tailscale status command could not be stopped.');
          }
        } finally {
          child.stdout?.destroy();
          child.stderr?.destroy();
        }
      }).catch(cause => { throw new ServiceCommandCleanupError(cause); });
      void stopping.catch(error => {
        clearTimeout(timer);
        options.signal?.removeEventListener('abort', abort);
        child.unref();
        reject(error);
      });
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
      void Promise.resolve(stopping).then(() => {
        resolve(report(failure ?? (code === 0 ? parseStatus(Buffer.concat(stdout).toString('utf8'))
          : stderrState(Buffer.concat(stderr).toString('utf8')))));
      }, reject);
    });
    options.signal?.addEventListener('abort', abort, { once: true });
    if (options.signal?.aborted) abort();
  });
}

// parseTailscaleStatus, normalizeMagicDnsName, isTailscaleIpv4Address and buildTailscaleHttpsBaseUrl adapted
// from T3 Code packages/tailscale/src/tailscale.ts at 5a906583 (MIT, T3 Tools Inc.). Effect Schema is dropped.
export const DEFAULT_TAILSCALE_SERVE_PORT = 443;

export interface TailscaleStatus {
  readonly magicDnsName: string | null;
  readonly tailnetIpv4Addresses: readonly string[];
  /** `null` when the CLI reports no capability map (older CLI), so callers still attempt serve. */
  readonly httpsServeEnabled: boolean | null;
}

export function isTailscaleIpv4Address(address: string): boolean {
  if (!isIPv4(address)) return false;
  const [first, second] = address.split('.').map(Number);
  return first === 100 && second! >= 64 && second! <= 127;
}

function normalizeMagicDnsName(dnsName: unknown): string | null {
  if (typeof dnsName !== 'string') return null;
  const normalized = dnsName.trim().replace(/\.$/u, '');
  return normalized.length > 0 ? normalized : null;
}

/** Throws on JSON that is not a status object, as T3's TailscaleStatusParseError does. */
export function parseTailscaleStatus(rawStatusJson: string): TailscaleStatus {
  const parsed: unknown = JSON.parse(rawStatusJson);
  if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error('invalid status');
  const self = (parsed as { Self?: unknown }).Self;
  const fields = self !== null && typeof self === 'object' ? self as Record<string, unknown> : {};
  const rawIps = fields.TailscaleIPs;
  const tailnetIpv4Addresses = Array.isArray(rawIps)
    ? rawIps.filter((address): address is string => typeof address === 'string' && isTailscaleIpv4Address(address)) : [];
  const capMap = fields.CapMap;
  return { magicDnsName: normalizeMagicDnsName(fields.DNSName), tailnetIpv4Addresses,
    httpsServeEnabled: typeof capMap === 'object' && capMap !== null && !Array.isArray(capMap)
      ? Object.hasOwn(capMap, 'https') : null };
}

export function buildTailscaleHttpsBaseUrl(input: { magicDnsName: string; servePort?: number }): string {
  const url = new URL(`https://${input.magicDnsName}`);
  const servePort = input.servePort ?? DEFAULT_TAILSCALE_SERVE_PORT;
  if (servePort !== DEFAULT_TAILSCALE_SERVE_PORT) url.port = String(servePort);
  url.pathname = '/';
  return url.toString();
}
