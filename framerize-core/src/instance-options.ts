import type { InferenceBackend } from './backend.js';
import type { Route } from './http-policy.js';

export interface RequestEvent {
  readonly requestId: string;
  readonly route: Route | 'unknown';
  readonly status: number;
  readonly outcome: 'completed' | 'failed' | 'cancelled';
  readonly durationMs: number;
}

export interface InstanceOptions {
  readonly backend: InferenceBackend;
  readonly token: string | null;
  readonly host?: string;
  readonly port?: number;
  readonly maxBodyBytes?: number;
  readonly headersTimeoutMs?: number;
  readonly inferenceTimeoutMs?: number;
  readonly idleTimeoutMs?: number;
  readonly requestHeadersTimeoutMs?: number;
  readonly shutdownTimeoutMs?: number;
  readonly onRequest?: (event: RequestEvent) => void;
}

export function instanceOptions(options: InstanceOptions) {
  try {
    const allowed = ['backend', 'token', 'host', 'port', 'maxBodyBytes', 'headersTimeoutMs',
      'inferenceTimeoutMs', 'idleTimeoutMs', 'requestHeadersTimeoutMs', 'shutdownTimeoutMs', 'onRequest'];
    if (Object.keys(options).some((name) => !allowed.includes(name))) throw new Error();
    const config = { host: '127.0.0.1', port: 0, maxBodyBytes: 16 * 1024 * 1024,
      headersTimeoutMs: 15_000, idleTimeoutMs: 60_000, requestHeadersTimeoutMs: 15_000,
      inferenceTimeoutMs: 300_000,
      shutdownTimeoutMs: 5_000, ...options };
    if (!config.backend || typeof config.backend.forward !== 'function'
      || typeof config.backend.ready !== 'function' || typeof config.backend.close !== 'function'
      || (config.token !== null && (typeof config.token !== 'string'
        || !/^[A-Za-z0-9._~+/-]+=*$/.test(config.token)))
      || typeof config.host !== 'string' || !config.host || /[\s\0/]/.test(config.host)) throw new Error();
    for (const name of ['port', 'maxBodyBytes', 'headersTimeoutMs', 'inferenceTimeoutMs', 'idleTimeoutMs',
      'requestHeadersTimeoutMs', 'shutdownTimeoutMs'] as const) {
      const value = config[name];
      if (!Number.isSafeInteger(value) || value < (name === 'port' ? 0 : 1)
        || value > (name === 'port' ? 65_535 : 2_147_483_647)) throw new Error();
    }
    if (config.onRequest !== undefined && typeof config.onRequest !== 'function') throw new Error();
    return Object.freeze(config);
  } catch { throw new Error('Invalid instance configuration'); }
}
