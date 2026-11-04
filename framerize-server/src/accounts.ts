import { open } from 'node:fs/promises';
import type { FileHandle } from 'node:fs/promises';
import { constants } from 'node:fs';
import { dirname, isAbsolute, resolve } from 'node:path';
import { AccountFileError, AccountSelectionError, ConfigurationError } from './errors.js';

export interface AccountMetadata {
  readonly id: string;
  readonly enabled?: boolean;
  readonly label?: string;
  readonly model?: string;
  readonly models?: readonly string[];
  readonly projectId?: string;
  readonly session?: AccountSessionReference;
}

export type AccountSessionReference = Readonly<
  { env: string; file?: never } | { file: string; env?: never }
>;

export interface FramerAccount extends AccountMetadata {
  readonly projectId: string;
  readonly session: AccountSessionReference;
}

export async function loadAccounts(path?: string): Promise<readonly AccountMetadata[]> {
  try { return await readAccounts(path); }
  catch (error) {
    if (error instanceof ConfigurationError) throw new AccountFileError(error.message);
    throw error;
  }
}

async function readAccounts(path?: string): Promise<readonly AccountMetadata[]> {
  if (!path) return Object.freeze([]);
  let file: FileHandle | undefined;
  let value: unknown;
  try {
    file = await open(path, constants.O_RDONLY | constants.O_NONBLOCK);
    const info = await file.stat();
    if (!info.isFile() || info.size > 1024 * 1024) throw new Error();
    const buffer = Buffer.alloc(1024 * 1024 + 1);
    let length = 0;
    while (length < buffer.length) {
      const { bytesRead } = await file.read(buffer, length, buffer.length - length, null);
      if (!bytesRead) break;
      length += bytesRead;
    }
    if (length > 1024 * 1024) throw new Error();
    value = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(buffer.subarray(0, length)));
  } catch { throw new ConfigurationError('Account metadata unavailable'); }
  finally { await file?.close(); }
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
      ...(account.session !== undefined ? { session: sessionReference(account.session, path) } : {}),
    });
  }));
}

function sessionReference(value: unknown, source: string): AccountSessionReference {
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

export async function loadFramerAccount(path: string, accountId?: string): Promise<FramerAccount> {
  if (accountId !== undefined && (typeof accountId !== 'string'
      || !/^[a-zA-Z0-9][a-zA-Z0-9._-]{0,63}$/.test(accountId))) {
    throw new ConfigurationError('Invalid account selector');
  }
  const accounts = (await loadAccounts(path)).filter(account => account.enabled !== false
    && (accountId === undefined || account.id === accountId));
  const selected = accounts[0];
  if (accounts.length !== 1 || !selected?.projectId || !selected.session) {
    throw new AccountSelectionError('Select one configured Framer account');
  }
  return selected as FramerAccount;
}

export async function readAccountSession(reference: AccountSessionReference,
  env: Readonly<Record<string, string | undefined>> = process.env, signal?: AbortSignal): Promise<string> {
  try {
    signal?.throwIfAborted();
    if (!reference || Object.keys(reference).length !== 1) throw new Error();
    const variable = reference.env;
    const value = typeof variable === 'string' && /^[A-Za-z_][A-Za-z0-9_]*$/.test(variable)
      ? env[variable] : variable === undefined ? await readSessionFile(reference.file, signal) : undefined;
    signal?.throwIfAborted();
    if (typeof value !== 'string' || !value || value.length > 16_384 ||
        value !== value.trim() || !/^[\x20-\x7e]+$/.test(value)) throw new Error();
    return value;
  } catch { throw new ConfigurationError('Account session unavailable'); }
}

async function readSessionFile(path: string | undefined, signal?: AbortSignal): Promise<string> {
  if (typeof path !== 'string' || !isAbsolute(path) || /[\x00-\x1f\x7f]/.test(path)) throw new Error();
  const file = await open(path, constants.O_RDONLY | constants.O_NONBLOCK);
  try {
    const info = await file.stat();
    if (!info.isFile() || info.size > 16_384 || (process.platform !== 'win32' && (info.mode & 0o077))) {
      throw new Error();
    }
    const buffer = Buffer.alloc(16_385);
    let length = 0;
    while (length < buffer.length) {
      signal?.throwIfAborted();
      const { bytesRead } = await file.read(buffer, length, buffer.length - length, null);
      if (!bytesRead) break;
      length += bytesRead;
    }
    if (length > 16_384) throw new Error();
    return new TextDecoder('utf-8', { fatal: true, ignoreBOM: true })
      .decode(buffer.subarray(0, length)).replace(/\r?\n$/, '');
  } finally { await file.close(); }
}
