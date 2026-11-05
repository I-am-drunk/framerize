// Account file edits and saved selection for the accounts and configure commands.
import { access, rm } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { parseAccountList, readAccountText, type AccountMetadata } from './accounts.js';
import { AccountCommandError, AccountFileError, ConfigurationError } from './errors.js';
import { loadConfig } from './config.js';
import { readSettings, serverHome, type SavedSettings } from './settings.js';
import { writeAtomic } from './service-control.js';

type Environment = Readonly<Record<string, string | undefined>>;
export type RawAccount = Record<string, unknown>;
export interface StorePaths {
  readonly settings: string;
  readonly accounts: string;
  readonly saved?: SavedSettings;
}

async function exists(path: string): Promise<boolean> {
  return access(path).then(() => true, (error: NodeJS.ErrnoException) => {
    if (error.code === 'ENOENT') return false;
    throw new ConfigurationError('Server settings are unavailable or invalid');
  });
}

export async function resolveStore(values: Readonly<Record<string, string>>,
  env: Environment): Promise<StorePaths> {
  const settings = resolve(values.config ?? join(serverHome(env), 'server.json'));
  const saved = await exists(settings) ? await readSettings(settings) : undefined;
  const savedPath = saved?.config.accountsFile;
  const named = values.accountsFile || env.FRAMERIZE_SERVER_ACCOUNTS
    || (typeof savedPath === 'string' && savedPath ? savedPath : undefined);
  return { settings, ...(saved ? { saved } : {}),
    accounts: resolve(named ?? join(serverHome(env), 'accounts.json')) };
}

// Keeps raw entries so relative file references stay relative, and the exact prior text for rollback.
export interface RawAccounts {
  readonly accounts: RawAccount[];
  readonly text: string | undefined;
}

export async function readRawAccounts(path: string): Promise<RawAccounts> {
  let text: string | undefined;
  let value: unknown = [];
  try {
    text = await readAccountText(path, true);
    if (text !== undefined) value = JSON.parse(text);
  } catch { throw new AccountFileError('Account metadata unavailable'); }
  checkAccounts(value, path);
  return { accounts: value as RawAccount[], text };
}

function checkAccounts(value: unknown, path: string): readonly AccountMetadata[] {
  try { return parseAccountList(value, path); }
  catch (error) { throw error instanceof ConfigurationError ? new AccountFileError(error.message) : error; }
}

// Validates before the atomic private write, so a rejected edit leaves the old file in place.
export async function writeAccounts(path: string, accounts: readonly RawAccount[],
  signal?: AbortSignal): Promise<void> {
  try { parseAccountList(accounts, path); }
  catch (error) { throw error instanceof ConfigurationError ? new AccountCommandError(error.message) : error; }
  signal?.throwIfAborted();
  await writeAtomic(path, `${JSON.stringify(accounts, null, 2)}\n`);
}

// Puts back the exact bytes read earlier, or removes a file this command created.
export async function restoreAccounts(path: string, previous: RawAccounts): Promise<void> {
  if (previous.text === undefined) await rm(path, { force: true });
  else await writeAtomic(path, previous.text);
}

export function findAccount(accounts: readonly RawAccount[], id: string | undefined): RawAccount {
  const account = accounts.find(entry => entry.id === id);
  if (!id || !account) throw new AccountCommandError('Unknown account id');
  return account;
}

// Saves the Framer selection in server.json so serve, check and up use it without flags.
export async function saveSettings(store: StorePaths, change: Readonly<Record<string, unknown>>,
  environment: Readonly<Record<string, string>> = {}, signal?: AbortSignal): Promise<SavedSettings> {
  const config: Record<string, unknown> = { ...store.saved?.config, ...change };
  for (const [name, value] of Object.entries(change)) if (value === undefined) delete config[name];
  const next: SavedSettings = { ...store.saved, version: 1, config,
    environment: { ...store.saved?.environment, ...environment } };
  loadConfig(config, next.environment);
  signal?.throwIfAborted();
  await writeAtomic(store.settings, `${JSON.stringify(next, null, 2)}\n`);
  return next;
}

export function selectedAccount(store: StorePaths): string | undefined {
  const { backend, accountId } = store.saved?.config ?? {};
  return backend === 'framer' && typeof accountId === 'string' && accountId ? accountId : undefined;
}
