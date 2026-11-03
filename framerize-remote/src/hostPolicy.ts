import { isIP } from 'node:net';

export function normalizeHostname(host: string): string {
  return host.toLowerCase().replace(/^\[|\]$/g, '').replace(/\.+$/u, '');
}

function parseIpv4(host: string): readonly number[] | null {
  if (isIP(host) !== 4) return null;
  const parts = host.split('.').map(Number);
  return parts.length === 4 && parts.every((part) => Number.isInteger(part) && part >= 0 && part <= 255)
    ? parts : null;
}

export function isLoopbackHost(host: string): boolean {
  const normalized = normalizeHostname(host);
  if (normalized === 'localhost' || normalized === '::1') return true;
  return parseIpv4(normalized)?.[0] === 127;
}
