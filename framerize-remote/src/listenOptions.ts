import type { ParsedArgs } from './cliArgs.js';
import { parsePersisted, type RemoteConfig } from './config.js';
import { normalizeHostname } from './hostPolicy.js';
import { RemoteDiagnosticError } from './diagnostics.js';

export const LISTEN_SPEC = { host: 'string', port: 'string', 'allow-non-loopback': 'boolean' } as const;

export interface ListenOverrides {
  readonly host?: string;
  readonly port?: number;
  readonly allowNonLoopback?: true;
}

export function parseListenOptions(options: ParsedArgs['options']): ListenOverrides {
  if (typeof options.port === 'string' && (!/^\d+$/.test(options.port)
    || !Number.isSafeInteger(Number(options.port)) || Number(options.port) > 65535)) {
    throw new RemoteDiagnosticError('listen_invalid');
  }
  return { ...(typeof options.host === 'string' ? { host: options.host } : {}),
    ...(typeof options.port === 'string' ? { port: Number(options.port) } : {}),
    ...(options['allow-non-loopback'] === true ? { allowNonLoopback: true } : {}) };
}

export function applyListenOverrides(config: RemoteConfig, overrides: ListenOverrides): RemoteConfig {
  const candidate = { ...config, host: overrides.host ?? config.host, port: overrides.port ?? config.port,
    allowNonLoopback: overrides.allowNonLoopback === true
      || (overrides.host === undefined && config.allowNonLoopback) };
  try { parsePersisted(candidate); }
  catch { throw new RemoteDiagnosticError('listen_invalid'); }
  return { ...candidate, host: normalizeHostname(candidate.host) };
}
