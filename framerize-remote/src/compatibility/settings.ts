import { createHash } from 'node:crypto';
import { lstat, mkdir, readFile, realpath, rename, rm, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { claudeEnv, type RemoteTarget } from './configure.js';

/** Claude Code reads user settings from CLAUDE_CONFIG_DIR, else ~/.claude. */
export function claudeSettingsPath(env: NodeJS.ProcessEnv, home: string): string {
  return join(env.CLAUDE_CONFIG_DIR || join(home, '.claude'), 'settings.json');
}

export const MANAGED_KEYS = ['ANTHROPIC_BASE_URL', 'ANTHROPIC_AUTH_TOKEN'] as const;
type Settings = Record<string, unknown> & { env?: Record<string, unknown> };
type Written = Partial<Record<(typeof MANAGED_KEYS)[number], string>>;
/** SHA-256 digests of the values Framerize wrote. The record never holds a credential. */
interface WriteRecord { readonly settingsPath: string; readonly written: Written }
const digest = (value: string): string => createHash('sha256').update(value).digest('hex');

export class SettingsError extends Error {}

async function settingsDestination(path: string): Promise<string> {
  try { return await realpath(path); }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') {
      throw new SettingsError('Claude settings could not be resolved.');
    }
    const entry = await lstat(path).catch((statError: NodeJS.ErrnoException) => {
      if (statError.code !== 'ENOENT') throw new SettingsError('Claude settings could not be read.');
      return undefined;
    });
    if (entry?.isSymbolicLink()) throw new SettingsError('Claude settings link has no target.');
    return path;
  }
}

async function readSettings(path: string): Promise<Settings | undefined> {
  let text: string;
  try { text = await readFile(path, 'utf8'); }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return undefined;
    throw new SettingsError('Claude settings could not be read.');
  }
  let value: unknown;
  try { value = JSON.parse(text); } catch { throw new SettingsError('Claude settings are not valid JSON.'); }
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new SettingsError('Claude settings must be a JSON object.');
  }
  const settings = value as Settings;
  if (settings.env !== undefined && (settings.env === null || typeof settings.env !== 'object' || Array.isArray(settings.env))) {
    throw new SettingsError('Claude settings env must be a JSON object.');
  }
  return settings;
}

async function writeAtomic(path: string, text: string): Promise<void> {
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  const temporary = `${path}.frm-${process.pid}-${Date.now()}`;
  try { await writeFile(temporary, text, { mode: 0o600, flag: 'wx' }); await rename(temporary, path); }
  finally { await rm(temporary, { force: true }); }
}

const recordPath = (stateDir: string): string => join(stateDir, 'claude-settings.json');

async function readRecord(stateDir: string): Promise<WriteRecord | undefined> {
  try {
    const value: unknown = JSON.parse(await readFile(recordPath(stateDir), 'utf8'));
    const record = value as Partial<WriteRecord> | null;
    return typeof record?.settingsPath === 'string' && record.written && typeof record.written === 'object'
      ? record as WriteRecord : undefined;
  } catch { return undefined; }
}

/** True when a current value is one Framerize wrote, judged by its recorded digest. */
function ours(record: WriteRecord | undefined, key: (typeof MANAGED_KEYS)[number], value: unknown): boolean {
  return typeof value === 'string' && record?.written[key] === digest(value);
}

/** Sets only the managed env keys and keeps every other setting. Values Framerize did not write stay. */
export async function writeClaudeSettings(settingsPath: string, stateDir: string, target: RemoteTarget): Promise<void> {
  settingsPath = await settingsDestination(settingsPath);
  const settings = await readSettings(settingsPath) ?? {};
  const env = { ...(settings.env ?? {}) };
  const record = await readRecord(stateDir);
  const values = claudeEnv(target);
  const foreign = MANAGED_KEYS.filter(key => env[key] !== undefined && !ours(record, key, env[key]));
  if (foreign.length > 0) {
    throw new SettingsError(`Claude settings already set ${foreign.join(' and ')}. Remove them or use frm client claude without --persist.`);
  }
  const written: Written = {};
  for (const key of MANAGED_KEYS) { written[key] = digest(values[key]!); env[key] = values[key]; }
  await writeAtomic(recordPath(stateDir), `${JSON.stringify({ settingsPath, written }, null, 2)}\n`);
  await writeAtomic(settingsPath, `${JSON.stringify({ ...settings, env }, null, 2)}\n`);
}

export type UndoResult = 'restored' | 'not_configured' | 'changed_since';

/** Removes only values that still match what Framerize wrote, then forgets the record. */
export async function undoClaudeSettings(stateDir: string): Promise<UndoResult> {
  const record = await readRecord(stateDir);
  if (!record) return 'not_configured';
  const settingsPath = await settingsDestination(record.settingsPath);
  const settings = await readSettings(settingsPath);
  const env = { ...(settings?.env ?? {}) };
  const changed = MANAGED_KEYS.some(key => env[key] !== undefined && !ours(record, key, env[key]));
  if (settings && !changed) {
    for (const key of MANAGED_KEYS) delete env[key];
    const { env: _previous, ...rest } = settings;
    const next = Object.keys(env).length > 0 ? { ...rest, env } : rest;
    await writeAtomic(settingsPath, `${JSON.stringify(next, null, 2)}\n`);
  }
  await rm(recordPath(stateDir), { force: true });
  return !settings ? 'not_configured' : changed ? 'changed_since' : 'restored';
}
