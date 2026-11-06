import { isIP } from 'node:net';
import { domainToASCII } from 'node:url';

export function normalizeHostname(host: string): string {
  const bare = host.replace(/^\[|\]$/g, '');
  const scope = bare.indexOf('%');
  if (scope >= 0 && isIP(bare) === 6) {
    return bare.slice(0, scope).toLowerCase() + bare.slice(scope);
  }
  return bare.toLowerCase().replace(/\.+$/u, '');
}

function parseIpv4(host: string): readonly number[] | null {
  const address = /^[\da-fx.]+$/i.test(host) ? domainToASCII(host) : host;
  if (isIP(address) !== 4) return null;
  const parts = address.split('.').map(Number);
  return parts.length === 4 && parts.every((part) => Number.isInteger(part) && part >= 0 && part <= 255)
    ? parts : null;
}

export function isLoopbackHost(host: string): boolean {
  const normalized = normalizeHostname(host);
  if (normalized === 'localhost' || normalized === '::1') return true;
  if (isIP(normalized) === 6) {
    try {
      const canonical = new URL(`http://[${normalized}]`).hostname;
      return canonical === '[::1]' || /^\[::ffff:7f[\da-f]{2}:[\da-f]{1,4}\]$/.test(canonical);
    } catch { return false; }
  }
  return parseIpv4(normalized)?.[0] === 127;
}
