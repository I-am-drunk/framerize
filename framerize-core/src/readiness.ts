export type ReadinessReason = 'configuration_unavailable'
  | 'credentials_unavailable' | 'upstream_unavailable';

export type Readiness = Readonly<
  | { state: 'ready' | 'malformed' | 'incompatible' }
  | { state: 'unavailable'; reason?: ReadinessReason }
>;

const API_VERSION = 1;
const CAPABILITIES = ['responses', 'chatCompletions', 'streaming'] as const;
const decoder = new TextDecoder('utf-8', { fatal: true });
const MALFORMED: Readiness = Object.freeze({ state: 'malformed' });

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function parse(bytes: unknown): unknown {
  if (!(bytes instanceof Uint8Array)) return undefined;
  try { return JSON.parse(decoder.decode(bytes)) as unknown; }
  catch { return undefined; }
}

function isReason(value: unknown): value is ReadinessReason {
  return value === 'configuration_unavailable' || value === 'credentials_unavailable'
    || value === 'upstream_unavailable';
}

function capabilityState(value: unknown): 'ready' | 'malformed' | 'incompatible' {
  if (!isRecord(value)) return 'malformed';
  const values = CAPABILITIES.map((name) => value[name]);
  if (values.some((capability) => typeof capability !== 'boolean')) return 'malformed';
  return values.every(Boolean) ? 'ready' : 'incompatible';
}

export function classifyReadiness(status: unknown, bytes: unknown): Readiness {
  if (typeof status !== 'number' || !Number.isInteger(status)
    || status < 100 || status > 599) return MALFORMED;
  const document = parse(bytes);
  if (!isRecord(document) || !Number.isInteger(document.apiVersion)) return MALFORMED;
  if (document.apiVersion !== API_VERSION) return Object.freeze({ state: 'incompatible' });
  const capabilities = capabilityState(document.capabilities);
  if (capabilities === 'malformed') return MALFORMED;
  if (document.status === 'unavailable') {
    return isReason(document.reason)
      ? Object.freeze({ state: 'unavailable', reason: document.reason }) : MALFORMED;
  }
  if (document.status !== 'ready' || Object.hasOwn(document, 'reason')) return MALFORMED;
  if (capabilities !== 'ready') return Object.freeze({ state: 'incompatible' });
  return Object.freeze({ state: status >= 200 && status < 300 ? 'ready' : 'unavailable' });
}
