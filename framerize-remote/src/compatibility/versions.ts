import { readFile } from 'node:fs/promises';
import type { ReadText } from './conflicts.js';
import { claudeManagedSettingsPath, parseSettingsObject } from './conflicts.js';

/** Retains at most 4096 response bytes, or returns null on any failure. */
export type FetchText = (url: string, timeoutMs: number, signal?: AbortSignal) => Promise<string | null>;

export const fetchText: FetchText = async (url, timeoutMs, signal) => {
  if (signal?.aborted) return null;
  let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
  try {
    const timeout = AbortSignal.timeout(timeoutMs);
    const response = await fetch(url, {
      signal: signal ? AbortSignal.any([signal, timeout]) : timeout, redirect: 'follow',
    });
    if (!response.ok) {
      await response.body?.cancel();
      return null;
    }
    if (!response.body) return '';
    reader = response.body.getReader();
    const chunks: Uint8Array[] = [];
    let size = 0;
    while (true) {
      const { done, value } = await reader.read();
      if (done) return new TextDecoder().decode(Buffer.concat(chunks, size));
      size += value.byteLength;
      if (size > 4096) return null;
      chunks.push(value);
    }
  } catch { return null; } finally {
    await reader?.cancel().catch(() => {});
    reader?.releaseLock();
  }
};

export const readText: ReadText = async path => {
  try { const text = await readFile(path, 'utf8'); return text.length <= 1024 * 1024 ? text : null; }
  catch { return null; }
};

/** Native release channel lookup adapted from T3 ClaudeMaintenance.readUpdateChannel at 5a906583 (MIT). */
export async function claudeUpdateChannel(read: ReadText, platform: NodeJS.Platform,
  env: NodeJS.ProcessEnv, userPath: string): Promise<'latest' | 'stable'> {
  let channel: 'latest' | 'stable' = 'latest';
  for (const path of [userPath, claudeManagedSettingsPath(platform, env)]) {
    const value = parseSettingsObject(await read(path))?.autoUpdatesChannel;
    if (value === 'latest' || value === 'stable') channel = value;
  }
  return channel;
}

export const claudeReleaseUrl = (channel: string): string =>
  `https://downloads.claude.ai/claude-code-releases/${channel}`;

const NATIVE_VERSION = /^\d+\.\d+\.\d+$/;
const NPM_NUMBER = '(?:0|[1-9][0-9]*)';
const NPM_PRERELEASE = `(?:${NPM_NUMBER}|[0-9]*[A-Za-z-][0-9A-Za-z-]*)`;
const NPM_VERSION = new RegExp(`^${NPM_NUMBER}\\.${NPM_NUMBER}\\.${NPM_NUMBER}`
  + `(?:-${NPM_PRERELEASE}(?:\\.${NPM_PRERELEASE})*)?`
  + '(?:\\+[0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*)?$');

/** npm view output, from T3 maintenanceResolver decodeNpmViewVersion: a string or a one-item array. */
export function parseNpmViewVersion(stdout: string | null): string | null {
  if (!stdout) return null;
  try {
    const value: unknown = JSON.parse(stdout.trim());
    if (Array.isArray(value) && value.length !== 1) return null;
    const version = Array.isArray(value) ? value[0] : value;
    if (typeof version !== 'string') return null;
    const normalized = version.trim();
    return NPM_VERSION.test(normalized) ? normalized : null;
  } catch { return null; }
}

/** From T3 parseHomebrewLatestVersion: a cask version before its comma; `latest` is unknown. */
export function parseHomebrewCaskVersion(stdout: string | null): string | null {
  if (!stdout) return null;
  try {
    const raw: unknown = (JSON.parse(stdout) as { casks?: { version?: unknown }[] }).casks?.[0]?.version;
    const version = typeof raw === 'string' ? raw.split(',', 1)[0]!.trim() : '';
    return version && version !== 'latest' ? version : null;
  } catch { return null; }
}

/** First x.y.z in `--version` output, such as `2.1.300 (Claude Code)` or `codex-cli 0.170.0`. */
export function parseInstalledVersion(stdout: string | null): string | null {
  return /\b(\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?)\b/.exec(stdout ?? '')?.[1] ?? null;
}

export const isNativeVersion = (text: string | null): text is string => NATIVE_VERSION.test(text?.trim() ?? '');

export type VersionStatus = 'behind' | 'current' | 'unknown';
export interface VersionCheck {
  readonly status: VersionStatus;
  readonly current: string | null;
  readonly latest: string | null;
}

/** From T3 deriveVersionAdvisory: a missing current or latest version is unknown, never current. */
export function versionStatus(current: string | null, latest: string | null,
  compare: (left: string, right: string) => number): VersionCheck {
  if (!current || !latest) return { status: 'unknown', current, latest };
  return { status: compare(current, latest) < 0 ? 'behind' : 'current', current, latest };
}

/** One line for humans. Unknown says why it is not reported as current. */
export function versionLine(check: VersionCheck): string {
  if (check.status === 'behind') return `Version ${check.current}; ${check.latest} is available.`;
  if (check.status === 'current') return `Version ${check.current} is the latest.`;
  return check.current ? `Version ${check.current}; the latest version could not be checked.`
    : 'The installed version could not be read.';
}
