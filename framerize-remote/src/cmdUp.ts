// `frm up` adapts T3 Code apps/server/src/cli/up.ts setUp at 5a906583 (MIT, T3 Tools Inc.):
// one command saves the connection, reconciles the background service, restarts it only when
// its settings changed, rolls the profile back when the service step fails, and prints the state.
import { readFile, rm, writeFile } from 'node:fs/promises';
import type { CliIo } from './cli.js';
import { runCheck } from './cmdRun.js';
import { CONFIG_DEFAULTS, parsePersisted, type PersistedConfig } from './config.js';
import { saveConfigFile } from './configStore.js';
import { validate, type CredentialReference } from './credentials.js';
import { FRM_LOCAL_TOKEN, FRM_SERVER_TOKEN, frmSettings } from './frmEnv.js';
import { RemoteDiagnosticError, type DiagnosticCode } from './diagnostics.js';
import { isPairingLink, parsePairingLink } from './pairingLink.js';
import { parseRemoteEndpoint } from './remoteUrl.js';
import { controlService, installService, requireServicePrerequisites, restoreService, serviceMainPid, serviceStatus,
  snapshotService, type ServiceContext, type ServiceSnapshot, type ServiceStatus } from './serviceControl.js';
import { checkLocalPort, waitForServiceListener } from './serviceReadiness.js';

export interface UpOptions {
  readonly configPath: string;
  readonly service: ServiceContext;
  readonly env: NodeJS.ProcessEnv;
  /** Bound on waiting for the managed listener, default 15000 ms. */
  readonly readyTimeoutMs?: number;
}

type Connection = { remote: string; token?: string | undefined; paired: boolean };

/** A pairing link, a server origin, FRM_REMOTE, the saved remote, or one terminal question. */
async function connection(input: string | undefined, saved: PersistedConfig | undefined,
  io: CliIo, env: NodeJS.ProcessEnv): Promise<Connection> {
  let value = input ?? (env.FRM_REMOTE || undefined) ?? saved?.remote;
  if (value === undefined && io.ask) value = (await io.ask('Pairing link or server address: ')).trim();
  if (!value) throw new RemoteDiagnosticError('remote_required');
  if (!isPairingLink(value)) {
    try { parseRemoteEndpoint(value, saved?.allowInsecureRemote === true); }
    catch { throw new RemoteDiagnosticError('remote_invalid'); }
    return { remote: value.replace(/\/$/, ''), paired: false };
  }
  const link = parsePairingLink(value);
  try { if (link.token !== undefined) validate(link.token); }
  catch { throw new RemoteDiagnosticError('pairing_link_invalid'); }
  return { remote: link.remote, token: link.token, paired: true };
}

async function readSaved(path: string): Promise<{ text: string | undefined; config: PersistedConfig | undefined }> {
  let text: string;
  try { text = await readFile(path, 'utf8'); }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return { text: undefined, config: undefined };
    throw new RemoteDiagnosticError('profile_invalid');
  }
  try { return { text, config: parsePersisted(JSON.parse(text)) }; }
  catch { throw new RemoteDiagnosticError('profile_invalid'); }
}

/** An FRM_* token saved by value, so the service needs no shell variable. Empty or malformed values fail. */
function envToken(env: NodeJS.ProcessEnv, name: string,
  code: 'server_credential' | 'local_credential'): CredentialReference | undefined {
  const value = env[name];
  if (value === undefined) return undefined;
  try { return { kind: 'value', value: validate(value) }; }
  catch { throw new RemoteDiagnosticError(code); }
}

/**
 * The profile `frm up` saves. Server token: pairing-link token, then FRM_SERVER_TOKEN, then the saved
 * token unless a tokenless link replaced it. Local token: FRM_LOCAL_TOKEN, then the saved token.
 */
function nextProfile(saved: PersistedConfig | undefined, link: Connection, env: NodeJS.ProcessEnv): PersistedConfig {
  const { serverToken: savedServer, localToken: savedLocal, ...rest } = saved ?? { remote: link.remote };
  const frm = frmSettings(env);
  const port = frm.port ?? saved?.port ?? CONFIG_DEFAULTS.port;
  const serverToken = link.token !== undefined ? { kind: 'value' as const, value: link.token }
    : envToken(env, FRM_SERVER_TOKEN, 'server_credential') ?? (link.paired ? undefined : savedServer);
  const localToken = envToken(env, FRM_LOCAL_TOKEN, 'local_credential') ?? savedLocal;
  return { ...rest, remote: link.remote, port: port === 0 ? CONFIG_DEFAULTS.port : port,
    ...(frm.host ? { host: frm.host } : {}), ...(serverToken ? { serverToken } : {}),
    ...(localToken ? { localToken } : {}) };
}

/** The host a local client dials, as `frm serve` prints it: wildcards become loopback, IPv6 gets brackets. */
function urlHost(host: string): string {
  const bare = host.replace(/^\[|\]$/g, '');
  if (bare === '0.0.0.0') return '127.0.0.1';
  if (bare === '::') return '[::1]';
  return bare.includes(':') ? `[${bare}]` : bare;
}

/** Restores the profile as it was before `frm up`, as T3's restoreServiceEnv restores service.env. */
async function restore(path: string, text: string | undefined): Promise<void> {
  if (text === undefined) await rm(path, { force: true });
  else await writeFile(path, text, { mode: 0o600 });
}

export interface UpResult {
  readonly remote: string;
  readonly paired: boolean;
  readonly ready: boolean;
  readonly service: 'installed' | 'updated' | 'restarted' | 'unchanged' | 'unavailable';
  readonly localApi: string;
}

/** Readiness through the saved profile, before any service change, as T3 checks before reconciling. */
async function readiness(configPath: string, env: NodeJS.ProcessEnv): Promise<DiagnosticCode | undefined> {
  // Only the saved profile counts: FRM_* values were already resolved into it.
  const clean = Object.fromEntries(Object.entries(env)
    .filter(([name]) => !/^FRM_(REMOTE|HOST|PORT|SERVER_TOKEN|LOCAL_TOKEN)$/.test(name)));
  const result = await runCheck(configPath, true, clean);
  if (result.exitCode === 0) return undefined;
  const code: unknown = (JSON.parse(result.text) as { diagnostic?: { code?: unknown } }).diagnostic?.code;
  return typeof code === 'string' ? code as DiagnosticCode : 'upstream_unavailable';
}

export async function setUp(input: string | undefined, io: CliIo, options: UpOptions): Promise<UpResult> {
  const before = await readSaved(options.configPath);
  const link = await connection(input, before.config, io, options.env);
  const profile = nextProfile(before.config, link, options.env);
  await saveConfigFile(options.configPath, profile);
  const after = await readFile(options.configPath, 'utf8');
  const localApi = `http://${urlHost(profile.host ?? CONFIG_DEFAULTS.host)}:${profile.port}/v1`;
  const base = { remote: link.remote, paired: link.paired, localApi };
  let snapshot: ServiceSnapshot | undefined;
  try {
    const notReady = await readiness(options.configPath, options.env);
    if (notReady) throw new RemoteDiagnosticError(notReady);
    const status = await serviceStatus(options.service);
    if (!status.supported) return { ...base, ready: true, service: 'unavailable' };
    if (status.otherHome) throw new RemoteDiagnosticError('service_other_home');
    if (status.current && status.running) await requireServicePrerequisites(options.service);
    // A running service reads the profile only when it starts; an unchanged one must still answer.
    if (status.current && status.running && after === before.text) {
      await listener(profile, options);
      return { ...base, ready: true, service: 'unchanged' };
    }
    snapshot = await snapshotService(options.service);
    const service = await startService(status, profile, options);
    await listener(profile, options);
    return { ...base, ready: true, service };
  } catch (error) {
    await restore(options.configPath, before.text).catch(() => {});
    if (snapshot) await restoreService(options.service, snapshot).catch(() => {});
    throw error;
  }
}

/**
 * Installs or restarts the service with the port released and checked first, as the Server's
 * reconcileService runs checkPort between stop and activation.
 */
async function startService(status: ServiceStatus, profile: PersistedConfig,
  options: UpOptions): Promise<UpResult['service']> {
  const free = () => checkLocalPort(profile.host ?? CONFIG_DEFAULTS.host, profile.port ?? CONFIG_DEFAULTS.port);
  if (!status.current || !status.running) return installService(options.service, free);
  await controlService(options.service, 'stop');
  await free();
  await controlService(options.service, 'start');
  return 'restarted';
}

/** Returns only once the managed process itself answers on the saved host and port. */
async function listener(profile: PersistedConfig, options: UpOptions): Promise<void> {
  const host = urlHost(profile.host ?? CONFIG_DEFAULTS.host).replace(/^\[|\]$/g, '');
  await waitForServiceListener({ host, port: profile.port ?? CONFIG_DEFAULTS.port,
    service: async () => {
      const [status, pid] = await Promise.all([serviceStatus(options.service), serviceMainPid(options.service)]);
      return { running: status.running, pid };
    }, ...(options.readyTimeoutMs ? { timeoutMs: options.readyTimeoutMs } : {}) });
}

/** Adapted from T3 `formatSetup` and `formatNextSteps`: what took effect, then one next step. */
export function formatUp(result: UpResult): string {
  const service = { installed: 'installed and started', updated: 'repaired and started',
    restarted: 'restarted with the new settings', unchanged: 'running', unavailable: 'unavailable on this machine' };
  const rows = [['Server', `${result.remote}${result.paired ? ' · paired' : ''}`], ['Readiness', 'passed'],
    ['Service', service[result.service]], ['Local API', result.localApi]];
  const next = result.service === 'unavailable'
    ? 'Keep this command running:\n  frm serve' : 'Point your client at the local API, or run `frm client`.';
  return `This machine\n${rows.map(([label, value]) => `  ${label!.padEnd(12)}${value}`).join('\n')}\n\n${next}\n`;
}
