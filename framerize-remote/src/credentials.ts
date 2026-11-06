import { constants } from 'node:fs';
import { open } from 'node:fs/promises';

const MAX_TOKEN_BYTES = 8192;
const BEARER = /^[A-Za-z0-9._~+/-]+=*$/;

export type CredentialReference =
  | { readonly kind: 'env'; readonly name: string }
  | { readonly kind: 'file'; readonly path: string }
  /** Saved by `frm up <pair-link>` in the private profile, so no token file or retyping is needed. */
  | { readonly kind: 'value'; readonly value: string };

export function validate(token: string): string {
  if (typeof token !== 'string' || token.length > MAX_TOKEN_BYTES || !BEARER.test(token)) {
    throw new Error('credential is not a valid bearer token');
  }
  return token;
}

async function readTokenFile(path: string, signal?: AbortSignal): Promise<string> {
  let handle;
  try { handle = await open(path, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOCTTY); }
  catch { throw new Error('credential file could not be opened'); }
  const closeOnAbort = (): void => { void handle.close().catch(() => {}); };
  signal?.addEventListener('abort', closeOnAbort, { once: true });
  try {
    signal?.throwIfAborted();
    const info = await handle.stat();
    signal?.throwIfAborted();
    if (!info.isFile() || info.size > MAX_TOKEN_BYTES) throw new Error('credential file is not a small regular file');
    const buffer = Buffer.alloc(MAX_TOKEN_BYTES + 1);
    let total = 0;
    while (total < buffer.length) {
      const { bytesRead } = await handle.read(buffer, total, buffer.length - total, null);
      signal?.throwIfAborted();
      if (bytesRead === 0) break;
      total += bytesRead;
    }
    if (total > MAX_TOKEN_BYTES) throw new Error('credential file exceeds byte limit');
    const text = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(buffer.subarray(0, total));
    return validate(text.replace(/\r?\n$/, ''));
  } finally {
    signal?.removeEventListener('abort', closeOnAbort);
    await handle.close();
  }
}

export async function cancellableRead(pending: Promise<string>, signal?: AbortSignal): Promise<string> {
  if (!signal) return pending;
  let abort = (): void => {};
  try {
    return await Promise.race([pending, new Promise<never>((_resolve, reject) => {
      abort = () => reject(signal.reason);
      signal.addEventListener('abort', abort, { once: true });
      if (signal.aborted) abort();
    })]);
  } finally { signal.removeEventListener('abort', abort); }
}

export async function loadCredential(
  reference: CredentialReference,
  env: NodeJS.ProcessEnv = process.env,
  signal?: AbortSignal,
): Promise<string> {
  try {
    signal?.throwIfAborted();
    if (reference.kind === 'env') {
      const value = env[reference.name];
      if (value === undefined) throw new Error('credential environment variable is not set');
      return validate(value);
    }
    if (reference.kind === 'value') return validate(reference.value);
    const token = await cancellableRead(readTokenFile(reference.path, signal), signal);
    signal?.throwIfAborted();
    return token;
  } catch {
    signal?.throwIfAborted();
    throw new Error('credential is unavailable or invalid');
  }
}
