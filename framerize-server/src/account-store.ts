// Account file edits and saved selection for the accounts and configure commands.
import { access, mkdir, open, realpath, rm, type FileHandle } from 'node:fs/promises';
import { basename, dirname, join, resolve } from 'node:path';
import { ACCOUNT_FILE_MAX_BYTES, parseAccountList, readAccountText, type AccountMetadata } from './accounts.js';
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
  env: Environment, signal?: AbortSignal): Promise<StorePaths> {
  signal?.throwIfAborted();
  const settings = resolve(values.config ?? join(serverHome(env), 'server.json'));
  const saved = await exists(settings) ? await readSettings(settings, signal) : undefined;
  signal?.throwIfAborted();
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

export async function readRawAccounts(path: string, signal?: AbortSignal): Promise<RawAccounts> {
  let text: string | undefined;
  let value: unknown = [];
  try {
    text = await readAccountText(path, true, signal);
    if (text !== undefined) value = JSON.parse(text);
  } catch {
    signal?.throwIfAborted();
    throw new AccountFileError('Account metadata unavailable');
  }
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
  const text = `${JSON.stringify(accounts, null, 2)}\n`;
  if (Buffer.byteLength(text, 'utf8') > ACCOUNT_FILE_MAX_BYTES) {
    throw new AccountCommandError('Account file would exceed the 1 MiB limit');
  }
  signal?.throwIfAborted();
  await writeAtomic(path, text);
}

// Puts back the exact bytes read earlier, or removes a file this command created.
export async function restoreAccounts(path: string, previous: RawAccounts): Promise<void> {
  if (previous.text === undefined) await rm(path, { force: true });
  else await writeAtomic(path, previous.text);
}

// Keep the snapshot check, replacement and any rollback exclusive across CLI processes.
export async function withAccountLock<T>(path: string, operation: () => Promise<T>,
  signal?: AbortSignal): Promise<T> {
  signal?.throwIfAborted();
  const directory = dirname(resolve(path));
  await mkdir(directory, { recursive: true, mode: 0o700 });
  const lockPath = join(await realpath(directory), `.${basename(path)}.lock`);
  signal?.throwIfAborted();
  let handle: FileHandle;
  try { handle = await open(lockPath, 'wx', 0o600); }
  catch (error) {
    signal?.throwIfAborted();
    if ((error as NodeJS.ErrnoException).code === 'EEXIST') {
      throw new AccountCommandError('Another command is editing this accounts file. Retry after it finishes.');
    }
    throw new AccountCommandError('Could not lock the accounts file for editing.');
  }
  let result: { value: T } | undefined;
  let failure: unknown;
  try {
    signal?.throwIfAborted();
    await handle.writeFile(`${process.pid}\n`);
    signal?.throwIfAborted();
    result = { value: await operation() };
  } catch (error) { failure = error; }
  const failures: unknown[] = [];
  try { await handle.close(); } catch (error) { failures.push(error); }
  try { await rm(lockPath, { force: true }); } catch (error) { failures.push(error); }
  if (failures.length) {
    throw Object.assign(new AccountCommandError(
      'Could not release the accounts file lock. Check it before retrying.'),
    { cause: new AggregateError(result ? failures : [failure, ...failures]) });
  }
  if (result) return result.value;
  throw failure;
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
  const { backend, accountId, accountsFile } = store.saved?.config ?? {};
  return backend === 'framer' && typeof accountsFile === 'string'
    && resolve(accountsFile) === store.accounts && typeof accountId === 'string' && accountId
    ? accountId : undefined;
}
