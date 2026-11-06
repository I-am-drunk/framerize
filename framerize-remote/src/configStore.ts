import { constants } from 'node:fs';
import { mkdir, mkdtemp, open, rename, rm } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { setTimeout as delay } from 'node:timers/promises';
import { RemoteDiagnosticError } from './diagnostics.js';
import { cancellableRead, type CredentialReference } from './credentials.js';
import { normalizeHostname } from './hostPolicy.js';
import { parseRemoteEndpoint, type RemoteEndpoint } from './remoteUrl.js';
import { frmSettings } from './frmEnv.js';
import {
  CONFIG_DEFAULTS, ConfigValidationError, parsePersisted, type RemoteConfig, type PersistedConfig,
} from './config.js';

function absoluteReference(reference: CredentialReference, base: string): CredentialReference {
  return reference.kind === 'file' ? { kind: 'file', path: resolve(base, reference.path) } : { ...reference };
}

async function readProfile(path: string, signal?: AbortSignal): Promise<string> {
  const handle = await open(path, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOCTTY);
  const closeOnAbort = (): void => { void handle.close().catch(() => {}); };
  signal?.addEventListener('abort', closeOnAbort, { once: true });
  try {
    signal?.throwIfAborted();
    if (!(await handle.stat()).isFile()) throw new Error();
    signal?.throwIfAborted();
    return await handle.readFile({ encoding: 'utf8', signal });
  } finally {
    signal?.removeEventListener('abort', closeOnAbort);
    await handle.close();
  }
}

export async function readConfigText(path: string, signal?: AbortSignal): Promise<string> {
  signal?.throwIfAborted();
  const text = await cancellableRead(readProfile(path, signal), signal);
  signal?.throwIfAborted();
  return text;
}

export async function loadConfigFile(path: string, signal?: AbortSignal): Promise<RemoteConfig> {
  let text: string;
  try { text = await readConfigText(path, signal); }
  catch (error) {
    signal?.throwIfAborted();
    const code = (error as NodeJS.ErrnoException).code === 'ENOENT' ? 'profile_missing' : 'profile_invalid';
    throw new RemoteDiagnosticError(code, 'no configuration found; run `frm setup` first');
  }
  let json: unknown;
  try { json = JSON.parse(text); }
  catch { throw new RemoteDiagnosticError('profile_invalid', 'configuration file is not valid JSON'); }
  return normalizeConfig(parsePersisted(json), dirname(resolve(path)));
}

export function normalizeConfig(persisted: PersistedConfig, base: string): RemoteConfig {
  return {
    remote: persisted.remote,
    ...(persisted.serverToken ? { serverToken: absoluteReference(persisted.serverToken, base) } : {}),
    ...(persisted.localToken ? { localToken: absoluteReference(persisted.localToken, base) } : {}),
    host: normalizeHostname(persisted.host ?? CONFIG_DEFAULTS.host),
    port: persisted.port ?? CONFIG_DEFAULTS.port,
    inferenceTimeoutMs: persisted.inferenceTimeoutMs ?? CONFIG_DEFAULTS.inferenceTimeoutMs,
    allowInsecureRemote: persisted.allowInsecureRemote ?? false,
    allowNonLoopback: persisted.allowNonLoopback ?? false,
  };
}

async function replaceProfile(temporary: string, path: string): Promise<void> {
  for (let attempt = 0; ; attempt++) {
    try { await rename(temporary, path); return; }
    catch (error) {
      const code = typeof error === 'object' && error !== null && 'code' in error ? error.code : undefined;
      if (process.platform !== 'win32' || attempt === 5
        || (code !== 'EPERM' && code !== 'EACCES' && code !== 'EBUSY')) throw error;
      // Keep the old profile in place while a transient Windows replacement failure clears.
      await delay(20 * (attempt + 1));
    }
  }
}

export function validateConfigForSave(config: PersistedConfig): RemoteConfig {
  let valid: RemoteConfig;
  try { valid = normalizeConfig(parsePersisted(config), process.cwd()); }
  catch (error) {
    if (error instanceof ConfigValidationError) throw new RemoteDiagnosticError(error.code, error.message);
    throw error;
  }
  resolveConfig(valid);
  return valid;
}

export async function saveConfigFile(path: string, config: PersistedConfig): Promise<void> {
  const valid = validateConfigForSave(config);
  let staging: string | undefined;
  try {
    await mkdir(dirname(path), { recursive: true, mode: 0o700 });
    staging = await mkdtemp(join(dirname(path), '.remote-profile-'));
    const temporary = join(staging, 'profile.json');
    const handle = await open(temporary, 'wx', 0o600);
    try { await handle.writeFile(`${JSON.stringify(valid, null, 2)}\n`); await handle.sync(); }
    finally { await handle.close(); }
    await replaceProfile(temporary, path);
  } catch { throw new RemoteDiagnosticError('profile_write', 'could not write the configuration file'); }
  finally { if (staging) await rm(staging, { recursive: true, force: true }); }
}

export function resolveConfig(config: RemoteConfig): RemoteEndpoint {
  parsePersisted(config);
  if (!Number.isInteger(config.port) || config.port < 0 || config.port > 65535) {
    throw new Error('listen port must be between 0 and 65535');
  }
  return parseRemoteEndpoint(config.remote, config.allowInsecureRemote);
}

export function sameRemoteOrigin(first: string, second: string): boolean {
  try { return parseRemoteEndpoint(first, true).origin === parseRemoteEndpoint(second, true).origin; }
  catch { return false; }
}

/** FRM_* overrides saved values, except setup's just-saved remote and token references. */
export async function loadRunConfig(path: string, env: NodeJS.ProcessEnv,
  afterSetup = false, signal?: AbortSignal): Promise<RemoteConfig> {
  signal?.throwIfAborted();
  const settings = frmSettings(env);
  let saved: RemoteConfig | undefined;
  try { saved = await loadConfigFile(path, signal); }
  catch (error) {
    signal?.throwIfAborted();
    if (afterSetup || !(error instanceof RemoteDiagnosticError && error.code === 'profile_missing' && settings.remote)) throw error;
  }
  if (afterSetup && saved) {
    settings.remote = saved.remote;
    if (saved.serverToken) settings.serverToken = saved.serverToken;
    if (saved.localToken) settings.localToken = saved.localToken;
  }
  const merged = { ...saved, ...settings };
  if (saved && settings.remote !== undefined && settings.serverToken === undefined
    && !sameRemoteOrigin(saved.remote, settings.remote)) delete merged.serverToken;
  if (merged.remote === undefined) throw new RemoteDiagnosticError('profile_missing');
  try { return normalizeConfig(parsePersisted(merged), process.cwd()); }
  catch (error) {
    if (error instanceof ConfigValidationError) throw new RemoteDiagnosticError(error.code, error.message);
    throw error;
  }
}
