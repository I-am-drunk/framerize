import { open } from 'node:fs/promises';
import type { FileHandle } from 'node:fs/promises';
import { constants } from 'node:fs';
import { dirname, isAbsolute, resolve } from 'node:path';
import { AccountFileError, AccountSelectionError, ConfigurationError } from './errors.js';

export const ACCOUNT_FILE_MAX_BYTES = 1024 * 1024;

export interface AccountMetadata {
  readonly id: string;
  readonly enabled?: boolean;
  readonly label?: string;
  readonly model?: string;
  readonly models?: readonly string[];
  readonly projectId?: string;
  readonly session?: AccountSession;
}

export type AccountSessionReference = Readonly<
  { env: string; file?: never } | { file: string; env?: never }
>;

export type AccountSession = string | AccountSessionReference;

export interface FramerAccount extends AccountMetadata {
  readonly projectId: string;
  readonly session: AccountSession;
}

export async function loadAccounts(path?: string, signal?: AbortSignal): Promise<readonly AccountMetadata[]> {
  try {
    signal?.throwIfAborted();
    return await readAccounts(path, signal);
  }
  catch (error) {
    signal?.throwIfAborted();
    if (error instanceof ConfigurationError) throw new AccountFileError(error.message);
    throw error;
  }
}

async function readAccounts(path?: string, signal?: AbortSignal): Promise<readonly AccountMetadata[]> {
  if (!path) return Object.freeze([]);
  let value: unknown;
  try { value = JSON.parse((await readAccountText(path, false, signal))!); }
  catch { throw new ConfigurationError('Account metadata unavailable'); }
  return parseAccountList(value, path);
}

// Bounded nonblocking read of a regular file. A missing file is undefined only when allowed.
export async function readAccountText(path: string, allowMissing = false,
  signal?: AbortSignal): Promise<string | undefined> {
  signal?.throwIfAborted();
  return cancellableRead(readAccountFile(path, allowMissing, signal), signal);
}

async function cancellableRead<Value>(pending: Promise<Value>, signal?: AbortSignal): Promise<Value> {
  if (!signal) return pending;
  let abort = (): void => {};
  try {
    const result = await Promise.race([pending, new Promise<never>((_resolve, reject) => {
      abort = () => reject(signal.reason);
      signal.addEventListener('abort', abort, { once: true });
      if (signal.aborted) abort();
    })]);
    signal.throwIfAborted();
    return result;
  } finally { signal.removeEventListener('abort', abort); }
}

async function readAccountFile(path: string, allowMissing: boolean,
  signal?: AbortSignal): Promise<string | undefined> {
  let file: FileHandle | undefined;
  let closing: Promise<void> | undefined;
  const close = (): Promise<void> => file ? closing ??= file.close() : Promise.resolve();
  const abort = (): void => { void close().catch(() => {}); };
  try {
    try { file = await open(path, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOCTTY); }
    catch (error) {
      if (allowMissing && (error as NodeJS.ErrnoException).code === 'ENOENT') return undefined;
      throw error;
    }
    signal?.addEventListener('abort', abort, { once: true });
    signal?.throwIfAborted();
    const info = await file.stat();
    signal?.throwIfAborted();
    if (!info.isFile() || info.size > ACCOUNT_FILE_MAX_BYTES) throw new Error();
    const buffer = Buffer.alloc(ACCOUNT_FILE_MAX_BYTES + 1);
    let length = 0;
    while (length < buffer.length) {
      const { bytesRead } = await file.read(buffer, length, buffer.length - length, null);
      signal?.throwIfAborted();
      if (!bytesRead) break;
      length += bytesRead;
    }
    if (length > ACCOUNT_FILE_MAX_BYTES) throw new Error();
    return new TextDecoder('utf-8', { fatal: true }).decode(buffer.subarray(0, length));
  } catch {
    signal?.throwIfAborted();
    throw new ConfigurationError('Account metadata unavailable');
  } finally {
    signal?.removeEventListener('abort', abort);
    await close();
  }
}

export function parseAccountList(value: unknown, path: string): readonly AccountMetadata[] {
  if (!Array.isArray(value)) throw new ConfigurationError('accounts JSON must be an array');
  const ids = new Set<string>();
  return Object.freeze(value.map((entry: unknown, index): AccountMetadata => {
    if (!entry || Array.isArray(entry) || typeof entry !== 'object') {
      throw new ConfigurationError(`accounts[${index}] must be an object`);
    }
    const account = entry as Record<string, unknown>;
    const allowed = ['id', 'enabled', 'label', 'model', 'models', 'projectId', 'session'];
    if (Object.keys(account).some((key) => !allowed.includes(key))) {
      throw new ConfigurationError(`accounts[${index}] has an unknown field`);
    }
    if (typeof account.id !== 'string' || !/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$/.test(account.id)) {
      throw new ConfigurationError(`accounts[${index}].id must be a short safe string`);
    }
    if (ids.has(account.id)) throw new ConfigurationError('duplicate account id');
    ids.add(account.id);
    if (account.enabled !== undefined && typeof account.enabled !== 'boolean') {
      throw new ConfigurationError(`accounts[${index}].enabled must be boolean`);
    }
    for (const key of ['label', 'model']) {
      const field = account[key];
      if (field !== undefined && (typeof field !== 'string' || field.length > 256)) {
        throw new ConfigurationError('Invalid account metadata');
      }
    }
    if (account.models !== undefined && (!Array.isArray(account.models) || account.models.length > 100
      || account.models.some((model) => typeof model !== 'string' || !model || model.length > 256))) {
      throw new ConfigurationError('Invalid account models');
    }
    if (account.projectId !== undefined && (typeof account.projectId !== 'string'
      || !/^[A-Za-z0-9_-]{1,128}$/.test(account.projectId))) {
      throw new ConfigurationError('Invalid account project');
    }
    return Object.freeze({ ...account, id: account.id,
      ...(Array.isArray(account.models) ? { models: Object.freeze([...account.models]) } : {}),
      ...(account.session !== undefined ? { session: parseSession(account.session, path) } : {}),
    });
  }));
}

function validSession(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0 && value.length <= 16_384
    && value === value.trim() && /^[\x20-\x7e]+$/.test(value);
}

function parseSession(value: unknown, source: string): AccountSession {
  if (validSession(value)) return value;
  if (value && typeof value === 'object' && !Array.isArray(value) && Object.keys(value).length === 1) {
    if ('env' in value && typeof value.env === 'string' && /^[A-Za-z_][A-Za-z0-9_]*$/.test(value.env)) {
      return Object.freeze({ env: value.env });
    }
    if ('file' in value && typeof value.file === 'string' && value.file && value.file.length <= 4096
        && !/[\x00-\x1f\x7f]/.test(value.file)) {
      return Object.freeze({ file: resolve(dirname(resolve(source)), value.file) });
    }
  }
  throw new ConfigurationError('Invalid account session reference');
}

export async function loadFramerAccount(path: string, accountId?: string,
  signal?: AbortSignal): Promise<FramerAccount> {
  if (accountId !== undefined && (typeof accountId !== 'string'
      || !/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$/.test(accountId))) {
    throw new ConfigurationError('Invalid account selector');
  }
  const accounts = (await loadAccounts(path, signal)).filter(account => account.enabled !== false
    && (accountId === undefined || account.id === accountId));
  const selected = accounts[0];
  if (accounts.length !== 1 || !selected?.projectId || !selected.session) {
    throw new AccountSelectionError('Select one configured Framer account');
  }
  return selected as FramerAccount;
}

export async function readAccountSession(reference: AccountSession,
  env: Readonly<Record<string, string | undefined>> = process.env, signal?: AbortSignal): Promise<string> {
  try {
    signal?.throwIfAborted();
    if (typeof reference === 'string') {
      if (!validSession(reference)) throw new Error();
      return reference;
    }
    if (!reference || Object.keys(reference).length !== 1) throw new Error();
    const variable = reference.env;
    const value = typeof variable === 'string' && /^[A-Za-z_][A-Za-z0-9_]*$/.test(variable)
      ? env[variable] : variable === undefined
        ? await cancellableRead(readSessionFile(reference.file, signal), signal) : undefined;
    signal?.throwIfAborted();
    if (!validSession(value)) throw new Error();
    return value;
  } catch { throw new ConfigurationError('Account session unavailable'); }
}

async function readSessionFile(path: string | undefined, signal?: AbortSignal): Promise<string> {
  if (typeof path !== 'string' || !isAbsolute(path) || /[\x00-\x1f\x7f]/.test(path)) throw new Error();
  const file = await open(path, constants.O_RDONLY | constants.O_NONBLOCK | constants.O_NOCTTY);
  let closing: Promise<void> | undefined;
  const close = (): Promise<void> => closing ??= file.close();
  const abort = (): void => { void close().catch(() => {}); };
  signal?.addEventListener('abort', abort, { once: true });
  try {
    signal?.throwIfAborted();
    const info = await file.stat();
    signal?.throwIfAborted();
    if (!info.isFile() || info.size > 16_384 || (process.platform !== 'win32' && (info.mode & 0o077))) {
      throw new Error();
    }
    const buffer = Buffer.alloc(16_385);
    let length = 0;
    while (length < buffer.length) {
      signal?.throwIfAborted();
      const { bytesRead } = await file.read(buffer, length, buffer.length - length, null);
      signal?.throwIfAborted();
      if (!bytesRead) break;
      length += bytesRead;
    }
    if (length > 16_384) throw new Error();
    return new TextDecoder('utf-8', { fatal: true, ignoreBOM: true })
      .decode(buffer.subarray(0, length)).replace(/\r?\n$/, '');
  } finally {
    signal?.removeEventListener('abort', abort);
    await close();
  }
}
