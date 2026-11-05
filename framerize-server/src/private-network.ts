// Adapted from T3 Code packages/shared/src/hostClassification.ts and
// apps/server/src/startupAccess.ts at 5a906583fed3fa41dc806f2a19bd9d095537ed3e.
// MIT, T3 Tools Inc. IPv6 loopback and ULA use hostClassification's private ranges.
import { isIPv4, isIPv6 } from 'node:net';

const isPrivateIpv4Address = (parts: readonly number[]): boolean =>
  parts[0] === 0 ||
  parts[0] === 10 ||
  parts[0] === 127 ||
  (parts[0] === 100 && parts[1]! >= 64 && parts[1]! <= 127) ||
  (parts[0] === 172 && parts[1]! >= 16 && parts[1]! <= 31) ||
  (parts[0] === 192 && parts[1] === 168) ||
  (parts[0] === 169 && parts[1] === 254) ||
  (parts[0] === 198 && parts[1]! >= 18 && parts[1]! <= 19);

export const isPrivateNetworkAddress = (address: string): boolean => {
  const ipv4 = address.replace(/^::ffff:/i, '');
  if (address === '::1') return true;
  if (isIPv6(address) && !address.toLowerCase().startsWith('::ffff:')) {
    const first = Number.parseInt(address.split(':', 1)[0] ?? '', 16);
    return Number.isInteger(first) && (first & 0xfe00) === 0xfc00;
  }
  return isIPv4(ipv4) && !ipv4.startsWith('169.254.')
    && isPrivateIpv4Address(ipv4.split('.').map(Number));
};

export const acceptsPrivateNetworkConnection = (socket: {
  readonly localAddress?: string | undefined;
  readonly remoteAddress?: string | undefined;
}): boolean => [socket.localAddress, socket.remoteAddress].every(
  address => address !== undefined && isPrivateNetworkAddress(address),
);
