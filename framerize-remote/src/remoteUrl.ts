const RAW_ORIGIN = /^https?:\/\/[^/\\@?#\s\u0000-\u001f\u007f]+\/?$/i;

export interface RemoteEndpoint {
  readonly origin: string;
  readonly loopback: boolean;
}

function isNumericLoopback(hostname: string): boolean {
  const host = hostname.replace(/^\[|\]$/g, '').toLowerCase();
  if (host === '::1') return true;
  const parts = host.split('.');
  if (parts.length !== 4) return false;
  if (!parts.every((part) => /^\d{1,3}$/.test(part) && Number(part) <= 255)) return false;
  return parts[0] === '127';
}

export function parseRemoteEndpoint(value: string, allowInsecure: boolean): RemoteEndpoint {
  if (typeof value !== 'string' || !RAW_ORIGIN.test(value)) {
    throw new Error('remote endpoint must be an http(s) origin with no path');
  }
  let url: URL;
  try { url = new URL(value); } catch { throw new Error('remote endpoint is not a valid URL'); }
  if (url.username || url.password || url.search || url.hash || url.pathname !== '/') {
    throw new Error('remote endpoint must not carry a path, query, or credentials');
  }
  const loopback = isNumericLoopback(url.hostname);
  if (url.protocol === 'http:' && !loopback && !allowInsecure) {
    throw new Error('plain HTTP remote requires numeric loopback or explicit insecure consent');
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') {
    throw new Error('remote endpoint must use HTTP or HTTPS');
  }
  return { origin: url.origin, loopback };
}
