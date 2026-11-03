// Pure classifier for the framerize-server v1 `GET /ready` document.
// No I/O: callers pass the HTTP status and the raw response body bytes.
const API_VERSION = 1;
const CAPABILITIES = ['responses', 'chatCompletions', 'streaming'];
const REASONS = new Set(['configuration_unavailable', 'credentials_unavailable', 'upstream_unavailable']);
const decoder = new TextDecoder('utf-8', { fatal: true });

const result = (state, reason) => Object.freeze(reason ? { state, reason } : { state });
const MALFORMED = result('malformed');
const isRecord = (value) => value !== null && typeof value === 'object' && !Array.isArray(value);

function parse(bytes) {
  if (!(bytes instanceof Uint8Array)) return undefined;
  try { return JSON.parse(decoder.decode(bytes)); } catch { return undefined; }
}

function capabilityState(capabilities) {
  if (!isRecord(capabilities)) return 'malformed';
  const values = CAPABILITIES.map((name) => capabilities[name]);
  if (values.some((value) => typeof value !== 'boolean')) return 'malformed';
  return values.every(Boolean) ? 'ready' : 'incompatible';
}

// Returns a frozen { state } with state ready, unavailable, incompatible or
// malformed; unavailable documents add their allow-listed reason. The result
// never echoes body content, so it is safe to log.
export function classifyReadiness(status, bytes) {
  if (!Number.isInteger(status) || status < 100 || status > 599) return MALFORMED;
  const document = parse(bytes);
  if (!isRecord(document) || !Number.isInteger(document.apiVersion)) return MALFORMED;
  if (document.apiVersion !== API_VERSION) return result('incompatible');
  const capabilities = capabilityState(document.capabilities);
  if (capabilities === 'malformed') return MALFORMED;
  if (document.status === 'unavailable') {
    return REASONS.has(document.reason) ? result('unavailable', document.reason) : MALFORMED;
  }
  if (document.status !== 'ready' || Object.hasOwn(document, 'reason')) return MALFORMED;
  if (capabilities !== 'ready') return result('incompatible');
  return status >= 200 && status < 300 ? result('ready') : result('unavailable');
}
