import { realpathSync } from 'node:fs';
import type { RemoteTarget } from './configure.js';
import type { ExecutableProbe } from './platform.js';
import type { ProcessRunner } from './process.js';
import type { ReadText } from './conflicts.js';
import { fetchText, readText, type FetchText } from './versions.js';
import type { LookupDeps } from './latest.js';

export interface ClientIo {
  readonly stdout: (text: string) => void;
  readonly stderr: (text: string) => void;
}

/** Asks one question on a terminal. Absent when stdin or stdout is not a TTY. */
export type Prompt = (question: string, signal?: AbortSignal) => Promise<string | null>;

/** Every outside effect the command uses. The parent supplies real values; tests stub them. */
export interface ClientSetupDeps {
  readonly platform: NodeJS.Platform;
  readonly env: NodeJS.ProcessEnv;
  readonly home: string;
  /** Private Remote state directory for the Claude settings write record. */
  readonly stateDir: string;
  readonly target: RemoteTarget;
  readonly runner: ProcessRunner;
  readonly io: ClientIo;
  readonly prompt?: Prompt;
  /** Cancels pending prompts and version lookups, and prevents further setup operations. */
  readonly signal?: AbortSignal;
  readonly probe?: ExecutableProbe;
  readonly realPath?: (path: string) => string;
  /** Release lookups. Defaults to a 4 s bounded fetch; any failure is unknown. Tests inject it. */
  readonly fetch?: FetchText;
  /** Settings reads for conflicts and the Claude update channel. Defaults to the real file. */
  readonly read?: ReadText;
  /** Directory whose project `.claude/` settings a launch would load. */
  readonly cwd?: string;
}

export const EXIT = { ok: 0, failed: 1, usage: 2, needsConsent: 3 } as const;

export const defaultRealPath = (path: string): string => {
  try { return realpathSync(path); } catch { return path; }
};

/** Asks one question and accepts only the exact word. Anything else, or no terminal, declines. */
export async function confirm(prompt: Prompt | undefined, question: string, word: string,
  signal?: AbortSignal): Promise<boolean> {
  if (!prompt || signal?.aborted) return false;
  const answer = await prompt(`${question} Type ${word} to continue: `, signal);
  return !signal?.aborted && answer?.trim() === word;
}

/** A prompt only when both streams are terminals, so agents and pipes never block on input. */
export function terminalPrompt(input: NodeJS.ReadStream, output: NodeJS.WriteStream): Prompt | undefined {
  if (!input.isTTY || !output.isTTY) return undefined;
  return async (question, signal) => {
    const { createInterface } = await import('node:readline/promises');
    const reader = createInterface({ input, output });
    try { return await reader.question(question, signal ? { signal } : {}); }
    catch { return null; } finally { reader.close(); }
  };
}

/** Lookup boundaries from the setup deps. Defaults are the real fetch and file read; tests inject both. */
export function lookupDeps(deps: ClientSetupDeps): LookupDeps {
  return { platform: deps.platform, env: deps.env, home: deps.home, runner: deps.runner,
    fetch: deps.fetch ?? fetchText, read: deps.read ?? readText,
    ...(deps.signal ? { signal: deps.signal } : {}),
    ...(deps.probe ? { probe: deps.probe } : {}) };
}
