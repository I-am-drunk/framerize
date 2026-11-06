// Adapted from T3 Code apps/server/src/startupAccess.ts at
// 5a906583fed3fa41dc806f2a19bd9d095537ed3e. MIT, T3 Tools Inc.
import { networkInterfaces } from 'node:os';
import { isPrivateNetworkAddress } from './private-network.js';
import { QrCode } from './qr-code.js';

type NetworkInterfacesMap = ReturnType<typeof networkInterfaces>;
export const VIRTUAL_INTERFACE =
  /^(docker|br-|veth|virbr|vmnet|vboxnet|vEthernet|podman|cni|flannel|cali|lxcbr|lxdbr|bridge[1-9]\d\d)/;

export const isWildcardHost = (host: string): boolean =>
  host === '0.0.0.0' || host === '::' || host === '[::]';

export const formatHostForUrl = (host: string): string =>
  host.includes(':') && !host.startsWith('[') ? `[${host}]` : host;

const normalizeHost = (host: string): string =>
  host.startsWith('[') && host.endsWith(']') ? host.slice(1, -1) : host;

const isSharedAddressSpaceHost = (host: string): boolean => {
  const parts = host.split('.').map(Number);
  return parts[0] === 100 && parts[1]! >= 64 && parts[1]! <= 127;
};

export function resolveHeadlessConnectionString(host: string, port: number, options: {
  readonly interfaces?: NetworkInterfacesMap;
  readonly privateNetworkOnly?: boolean;
} = {}): string {
  let connectionHost = normalizeHost(host);
  if (isWildcardHost(host)) {
    const interfaces = options.interfaces ?? networkInterfaces();
    const entries = Object.entries(interfaces).flatMap(([name, addresses]) =>
      options.privateNetworkOnly && VIRTUAL_INTERFACE.test(name) ? [] : addresses ?? []);
    const ipv4 = entries.filter(entry => !entry.internal && entry.family === 'IPv4'
      && (!options.privateNetworkOnly || isPrivateNetworkAddress(entry.address)));
    const preferred = options.privateNetworkOnly
      ? ipv4.find(entry => !isSharedAddressSpaceHost(entry.address)) ?? ipv4[0] : ipv4[0];
    const ipv6 = host === '0.0.0.0' ? undefined
      : entries.find(entry => !entry.internal && entry.family === 'IPv6' && entry.scopeid === 0
        && (!options.privateNetworkOnly || isPrivateNetworkAddress(entry.address)));
    connectionHost = preferred?.address ?? ipv6?.address ?? 'localhost';
  }
  return `http://${formatHostForUrl(connectionHost)}:${port}`;
}

/** Pairing credentials travel in the fragment, never in an HTTP request target. */
export function buildPairingUrl(connectionString: string, token?: string | null): string {
  const url = new URL(connectionString);
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password) {
    throw new TypeError('Pairing requires an HTTP or HTTPS server origin');
  }
  if (url.pathname !== '/' || url.search || url.hash) {
    throw new TypeError('Pairing requires a server origin without a path, query or fragment');
  }
  url.pathname = '/pair';
  url.search = '';
  url.hash = token ? new URLSearchParams([['token', token]]).toString() : '';
  return url.toString();
}

export function renderTerminalQrCode(value: string, margin = 2): string {
  const qrCode = QrCode.encodeText(value, QrCode.Ecc.MEDIUM);
  const rows: string[] = [];
  const isDark = (x: number, y: number): boolean =>
    x >= 0 && x < qrCode.size && y >= 0 && y < qrCode.size && qrCode.getModule(x, y);
  for (let y = -margin; y < qrCode.size + margin; y += 2) {
    let row = '';
    for (let x = -margin; x < qrCode.size + margin; x += 1) {
      const topDark = isDark(x, y);
      const bottomDark = isDark(x, y + 1);
      row += topDark ? (bottomDark ? '█' : '▀') : bottomDark ? '▄' : ' ';
    }
    rows.push(row);
  }
  return rows.join('\n');
}
