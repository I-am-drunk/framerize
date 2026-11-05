import type { CredentialReference } from './credentials.js';
import { DEFAULT_BACKEND_TIMINGS } from './backendConfig.js';
import { isIP } from 'node:net';
import { domainToASCII } from 'node:url';

export class ConfigValidationError extends Error {
  constructor(readonly code: 'inference_invalid' | 'listen_invalid' | 'server_reference' | 'local_reference',
    message: string) { super(message); }
}

function isListenHost(host: string): boolean {
  const bare = host.replace(/^\[([^\]]+)\]$/, '$1');
  if (isIP(bare)) return true;
  if (/[\\/:@?#\[\]]/.test(host)) return false;
  const ascii = domainToASCII(host).replace(/\.$/, '');
  return ascii.length > 0 && ascii.length <= 253 && ascii.split('.').every((label) =>
    /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/i.test(label));
}

export interface RemoteConfig {
  readonly remote: string;
  readonly serverToken?: CredentialReference;
  readonly localToken?: CredentialReference;
  readonly host: string;
  readonly port: number;
  readonly inferenceTimeoutMs: number;
  readonly allowInsecureRemote: boolean;
  readonly allowNonLoopback: boolean;
}

export interface PersistedConfig {
  readonly remote: string;
  readonly serverToken?: CredentialReference;
  readonly localToken?: CredentialReference;
  readonly host?: string;
  readonly port?: number;
  readonly inferenceTimeoutMs?: number;
  readonly allowInsecureRemote?: boolean;
  readonly allowNonLoopback?: boolean;
}

export const CONFIG_DEFAULTS = { host: '127.0.0.1', port: 8788,
  inferenceTimeoutMs: DEFAULT_BACKEND_TIMINGS.inferenceTimeoutMs } as const;

function isReference(value: unknown): value is CredentialReference {
  if (value === null || typeof value !== 'object') return false;
  const ref = value as Record<string, unknown>;
  if (ref.kind === 'env') return typeof ref.name === 'string' && /^[A-Za-z_][A-Za-z0-9_]*$/.test(ref.name);
  if (ref.kind === 'file') return typeof ref.path === 'string' && ref.path.length > 0 && !/[\x00-\x1f\x7f]/.test(ref.path);
  if (ref.kind === 'value') return typeof ref.value === 'string' && ref.value.length <= 8192
    && /^[A-Za-z0-9._~+/-]+=*$/.test(ref.value);
  return false;
}

export function parsePersisted(value: unknown): PersistedConfig {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('configuration must be a JSON object');
  }
  const raw = value as Record<string, unknown>;
  if (raw.inferenceTimeoutMs !== undefined && (typeof raw.inferenceTimeoutMs !== 'number'
    || !Number.isSafeInteger(raw.inferenceTimeoutMs) || raw.inferenceTimeoutMs < 1
    || raw.inferenceTimeoutMs > 2_147_483_647 - DEFAULT_BACKEND_TIMINGS.readinessWholeMs)) {
    throw new ConfigValidationError('inference_invalid', 'configuration inference timeout is invalid');
  }
  for (const flag of ['allowInsecureRemote', 'allowNonLoopback']) {
    if (raw[flag] !== undefined && typeof raw[flag] !== 'boolean') {
      throw new Error('configuration consent must be boolean');
    }
  }
  if (raw.host !== undefined && (typeof raw.host !== 'string' || !raw.host || /[\s\x00-\x1f\x7f]/.test(raw.host) || !isListenHost(raw.host))) {
    throw new ConfigValidationError('listen_invalid', 'configuration host is invalid');
  }
  if (raw.port !== undefined && (typeof raw.port !== 'number' || !Number.isInteger(raw.port) || raw.port < 0 || raw.port > 65535)) {
    throw new ConfigValidationError('listen_invalid', 'configuration port must be between 0 and 65535');
  }
  if (typeof raw.remote !== 'string') throw new Error('configuration remote must be a string');
  if (raw.serverToken !== undefined && !isReference(raw.serverToken)) throw new ConfigValidationError('server_reference', 'configuration serverToken reference is invalid');
  if (raw.localToken !== undefined && !isReference(raw.localToken)) throw new ConfigValidationError('local_reference', 'configuration localToken reference is invalid');
  return raw as unknown as PersistedConfig;
}
