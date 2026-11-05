import { RemoteDiagnosticError } from './diagnostics.js';

export interface PairingLink {
  readonly remote: string;
  readonly token?: string;
}

/**
 * Parses `<origin>/pair#token=<URLSearchParams-encoded token>` from `framerize-server pair`.
 * The fragment mirrors T3 Code's buildPairingUrl, so the token never reaches request logs.
 * The token is decoded exactly; `frm up` checks the bearer grammar before saving it.
 */
export function parsePairingLink(input: string): PairingLink {
  let url: URL;
  try { url = new URL(input); } catch { throw new RemoteDiagnosticError('pairing_link_invalid'); }
  if ((url.protocol !== 'http:' && url.protocol !== 'https:') || url.username || url.password
    || url.search || url.pathname !== '/pair' || /\?/.test(input.split('#', 1)[0]!)) {
    throw new RemoteDiagnosticError('pairing_link_invalid');
  }
  const fragment = new URLSearchParams(url.hash.slice(1));
  const tokens = fragment.getAll('token');
  if (tokens.length > 1 || [...fragment.keys()].some((key) => key !== 'token')) {
    throw new RemoteDiagnosticError('pairing_link_invalid');
  }
  const token = tokens[0];
  if (token === undefined || token === '') return { remote: url.origin };
  return { remote: url.origin, token };
}

/** A pairing link, as opposed to a bare server origin. */
export function isPairingLink(input: string): boolean {
  try { return new URL(input).pathname === '/pair'; } catch { return false; }
}
