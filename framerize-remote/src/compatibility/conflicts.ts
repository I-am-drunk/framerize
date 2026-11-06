import { join } from 'node:path';
import type { ClientCommand } from './clients.js';

/** Reads a small text file, or null when absent or unreadable. Tests inject synthetic files. */
export type ReadText = (path: string) => Promise<string | null>;

/**
 * Settings env values win over the launch environment, and a cloud provider flag outranks
 * ANTHROPIC_AUTH_TOKEN (code.claude.com/docs/en/llm-gateway-connect, /authentication).
 */
const PROVIDER_ENV = ['CLAUDE_CODE_USE_BEDROCK', 'CLAUDE_CODE_USE_VERTEX', 'CLAUDE_CODE_USE_FOUNDRY',
  'CLAUDE_CODE_USE_ANTHROPIC_AWS'];

export interface SettingsConflict { readonly path: string; readonly keys: readonly string[] }

/** Managed settings paths from T3 ClaudeMaintenance.readUpdateChannel at 5a906583 (MIT). */
export function claudeManagedSettingsPath(platform: NodeJS.Platform, env: NodeJS.ProcessEnv): string {
  const programFiles = env.PROGRAMFILES
    ?? Object.entries(env).find(([name]) => name.toUpperCase() === 'PROGRAMFILES')?.[1];
  const directory = platform === 'darwin' ? '/Library/Application Support/ClaudeCode'
    : platform === 'win32' ? join(programFiles || 'C:\\Program Files', 'ClaudeCode') : '/etc/claude-code';
  return join(directory, 'managed-settings.json');
}

export function parseSettingsObject(text: string | null): Record<string, unknown> | null {
  if (text === null) return null;
  try {
    const value: unknown = JSON.parse(text);
    return value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null;
  } catch { return null; }
}

/** Lists keys in one settings file that would route Claude away from the launch values. */
function conflictKeys(settings: Record<string, unknown> | null, managed: boolean,
  expected: Readonly<Record<string, string>>, platform: NodeJS.Platform): string[] {
  const raw = settings?.env;
  const env = raw !== null && typeof raw === 'object' && !Array.isArray(raw) ? raw as Record<string, unknown> : {};
  const names = Object.keys(env);
  const matching = (key: string): string[] => names.filter(name =>
    (platform === 'win32' ? name.toUpperCase() : name) === key && env[name] !== undefined);
  const differing = (key: string): string[] => matching(key).filter(name => env[name] !== expected[key]);
  return [
    ...['ANTHROPIC_BASE_URL', 'ANTHROPIC_AUTH_TOKEN'].flatMap(differing).map(key => `env.${key}`),
    ...['ANTHROPIC_API_KEY', ...PROVIDER_ENV].flatMap(matching).map(key => `env.${key}`),
    ...(settings?.apiKeyHelper !== undefined ? ['apiKeyHelper'] : []),
    ...(managed ? ['forceLoginMethod', 'forceLoginOrgUUID'].filter(key => settings?.[key] !== undefined) : []),
  ];
}

/** The `--settings` launch argument only replaces a foreign base URL; it never carries a token. */
export interface LaunchSettings {
  readonly overrideBaseUrl: boolean;
  readonly blocking: readonly SettingsConflict[];
}

export async function readSettings(read: ReadText, path: string, signal?: AbortSignal): Promise<string | null> {
  if (!signal) return read(path);
  signal.throwIfAborted();
  let abort = (): void => {};
  try {
    const cancelled = new Promise<never>((resolveUnused, reject) => {
      abort = () => reject(signal.reason);
      signal.addEventListener('abort', abort, { once: true });
      if (signal.aborted) abort();
    });
    const text = await Promise.race([Promise.resolve().then(() => {
      signal.throwIfAborted();
      return read(path);
    }), cancelled]);
    signal.throwIfAborted();
    return text;
  } finally {
    signal.removeEventListener('abort', abort);
  }
}

/** Inspects user, project, local and managed Claude settings before a launch. */
export async function inspectClaudeSettings(read: ReadText, input: { readonly platform: NodeJS.Platform;
  readonly env: NodeJS.ProcessEnv; readonly userPath: string; readonly cwd: string;
  readonly localPath?: string;
  readonly signal?: AbortSignal;
  readonly expected: Readonly<Record<string, string>> }): Promise<LaunchSettings> {
  const managedPath = claudeManagedSettingsPath(input.platform, input.env);
  const localPath = join(input.cwd, '.claude', 'settings.local.json');
  const files = new Set([input.userPath, join(input.cwd, '.claude', 'settings.json'),
    localPath, input.localPath ?? localPath, managedPath]);
  let overrideBaseUrl = false;
  const blocking: SettingsConflict[] = [];
  for (const path of files) {
    const managed = path === managedPath;
    const text = await readSettings(read, path, input.signal);
    const keys = conflictKeys(parseSettingsObject(text), managed, input.expected, input.platform);
    const blocked = keys.filter(key => managed || key.toUpperCase() !== 'ENV.ANTHROPIC_BASE_URL');
    if (keys.length > blocked.length) overrideBaseUrl = true;
    if (blocked.length > 0) blocking.push({ path, keys: blocked });
  }
  return { overrideBaseUrl, blocking };
}

/** Names each conflicting key and its file. Values stay out of the message. */
export function conflictText(blocking: readonly SettingsConflict[]): string {
  const lines = blocking.map(conflict => `  ${conflict.path}: ${conflict.keys.join(', ')}`);
  return `Claude settings would send this session somewhere other than Remote, so it was not started:\n${lines.join('\n')}\n`
    + 'Remove those keys, or run claude with them yourself if that route is intended.\n'
    + 'If frm client claude --persist wrote them for another Remote, run frm client claude --undo.\n';
}

/** Adds `--settings` with only the base URL; the token stays in the child environment. */
export function withSettingsOverride(launch: ClientCommand, baseUrl: string): ClientCommand {
  const settings = JSON.stringify({ env: { ANTHROPIC_BASE_URL: baseUrl } });
  return { ...launch, args: ['--settings', settings, ...launch.args],
    display: `claude --settings '${settings}' ${launch.args.join(' ')}`.trimEnd() };
}
