import { mkdir, mkdtemp, open, readFile, rename, rm } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { RemoteDiagnosticError } from './diagnostics.js';
import type { CredentialReference } from './credentials.js';
import { isLoopbackHost, normalizeHostname } from './hostPolicy.js';
import { parseRemoteEndpoint, type RemoteEndpoint } from './remoteUrl.js';
import {
  CONFIG_DEFAULTS, ConfigValidationError, parsePersisted, type RemoteConfig, type PersistedConfig,
} from './config.js';

function absoluteReference(reference: CredentialReference, base: string): CredentialReference {
  return reference.kind === 'file' ? { kind: 'file', path: resolve(base, reference.path) }
    : { kind: 'env', name: reference.name };
}

export async function loadConfigFile(path: string): Promise<RemoteConfig> {
  let text: string;
  try { text = await readFile(path, 'utf8'); }
  catch (error) {
    const code = (error as NodeJS.ErrnoException).code === 'ENOENT' ? 'profile_missing' : 'profile_invalid';
    throw new RemoteDiagnosticError(code, 'no configuration found; run `framerize-remote setup` first');
  }
  let json: unknown;
  try { json = JSON.parse(text); }
  catch { throw new RemoteDiagnosticError('profile_invalid', 'configuration file is not valid JSON'); }
  return normalizeConfig(parsePersisted(json), dirname(resolve(path)));
}

function normalizeConfig(persisted: PersistedConfig, base: string): RemoteConfig {
  return {
    remote: persisted.remote,
    serverToken: absoluteReference(persisted.serverToken, base),
    localToken: absoluteReference(persisted.localToken, base),
    host: normalizeHostname(persisted.host ?? CONFIG_DEFAULTS.host),
    port: persisted.port ?? CONFIG_DEFAULTS.port,
    inferenceTimeoutMs: persisted.inferenceTimeoutMs ?? CONFIG_DEFAULTS.inferenceTimeoutMs,
    allowInsecureRemote: persisted.allowInsecureRemote ?? false,
    allowNonLoopback: persisted.allowNonLoopback ?? false,
  };
}

export async function saveConfigFile(path: string, config: PersistedConfig): Promise<void> {
  let valid: RemoteConfig;
  try { valid = normalizeConfig(parsePersisted(config), process.cwd()); }
  catch (error) {
    if (error instanceof ConfigValidationError) throw new RemoteDiagnosticError(error.code, error.message);
    throw error;
  }
  resolveConfig(valid);
  let staging: string | undefined;
  try {
    await mkdir(dirname(path), { recursive: true, mode: 0o700 });
    staging = await mkdtemp(join(dirname(path), '.remote-profile-'));
    const temporary = join(staging, 'profile.json');
    const handle = await open(temporary, 'wx', 0o600);
    try { await handle.writeFile(`${JSON.stringify(valid, null, 2)}\n`); await handle.sync(); }
    finally { await handle.close(); }
    await rename(temporary, path);
  } catch { throw new RemoteDiagnosticError('profile_write', 'could not write the configuration file'); }
  finally { if (staging) await rm(staging, { recursive: true, force: true }); }
}

export function resolveConfig(config: RemoteConfig): RemoteEndpoint {
  parsePersisted(config);
  if (!isLoopbackHost(config.host) && !config.allowNonLoopback) {
    throw new RemoteDiagnosticError('exposure_consent', 'non-loopback listen host requires explicit exposure consent');
  }
  if (!Number.isInteger(config.port) || config.port < 0 || config.port > 65535) {
    throw new Error('listen port must be between 0 and 65535');
  }
  return parseRemoteEndpoint(config.remote, config.allowInsecureRemote);
}
