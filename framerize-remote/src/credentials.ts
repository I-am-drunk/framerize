import { constants } from 'node:fs';
import { open } from 'node:fs/promises';

const MAX_TOKEN_BYTES = 8192;
const BEARER = /^[A-Za-z0-9._~+/-]+=*$/;

export type CredentialReference =
  | { readonly kind: 'env'; readonly name: string }
  | { readonly kind: 'file'; readonly path: string };

function validate(token: string): string {
  if (typeof token !== 'string' || token.length > MAX_TOKEN_BYTES || !BEARER.test(token)) {
    throw new Error('credential is not a valid bearer token');
  }
  return token;
}

async function readTokenFile(path: string): Promise<string> {
  let handle;
  try { handle = await open(path, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOCTTY); }
  catch { throw new Error('credential file could not be opened'); }
  try {
    const info = await handle.stat();
    if (!info.isFile() || info.size > MAX_TOKEN_BYTES) throw new Error('credential file is not a small regular file');
    const buffer = Buffer.alloc(MAX_TOKEN_BYTES + 1);
    let total = 0;
    while (total < buffer.length) {
      const { bytesRead } = await handle.read(buffer, total, buffer.length - total, null);
      if (bytesRead === 0) break;
      total += bytesRead;
    }
    if (total > MAX_TOKEN_BYTES) throw new Error('credential file exceeds byte limit');
    const text = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(buffer.subarray(0, total));
    return validate(text.replace(/\r?\n$/, ''));
  } finally {
    await handle.close();
  }
}

export async function loadCredential(
  reference: CredentialReference,
  env: NodeJS.ProcessEnv = process.env,
): Promise<string> {
  try {
    if (reference.kind === 'env') {
      const value = env[reference.name];
      if (value === undefined) throw new Error('credential environment variable is not set');
      return validate(value);
    }
    return await readTokenFile(reference.path);
  } catch {
    throw new Error('credential is unavailable or invalid');
  }
}
