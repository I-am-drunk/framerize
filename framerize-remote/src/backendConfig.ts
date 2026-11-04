import type { RemoteEndpoint } from './remoteUrl.js';

export interface RemoteBackendConfig {
  readonly endpoint: RemoteEndpoint;
  readonly serverToken: string;
  readonly headersTimeoutMs: number;
  readonly inferenceTimeoutMs?: number;
  readonly idleTimeoutMs: number;
  readonly readinessBodyMaxBytes: number;
  readonly readinessWholeMs: number;
}

export const DEFAULT_BACKEND_TIMINGS = {
  headersTimeoutMs: 15_000,
  inferenceTimeoutMs: 300_000,
  idleTimeoutMs: 60_000,
  readinessBodyMaxBytes: 64 * 1024,
  readinessWholeMs: 20_000,
} as const;

export const FORWARDED_REQUEST_HEADERS = ['accept', 'content-type', 'openai-beta'] as const;
