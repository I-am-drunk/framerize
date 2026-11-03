import { open } from 'node:fs/promises';
import type { FileHandle } from 'node:fs/promises';
import { constants } from 'node:fs';
import { ConfigurationError } from './errors.js';

export interface AccountMetadata {
  readonly id: string;
  readonly enabled?: boolean;
  readonly label?: string;
  readonly model?: string;
  readonly models?: readonly string[];
}

export async function loadAccounts(path?: string): Promise<readonly AccountMetadata[]> {
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
    const allowed = ['id', 'enabled', 'label', 'model', 'models'];
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
    return Object.freeze({ ...account, id: account.id,
      ...(Array.isArray(account.models) ? { models: Object.freeze([...account.models]) } : {}),
    });
  }));
}
