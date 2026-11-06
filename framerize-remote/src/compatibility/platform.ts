// PATH extension defaults, command candidates and quote stripping adapted from T3 Code
// packages/shared/src/shell.ts at 5a906583 (MIT).
import { accessSync, constants, statSync } from 'node:fs';
import { posix, win32 } from 'node:path';

/** Reports whether a candidate path is an executable file. Tests replace it. */
export type ExecutableProbe = (candidate: string, platform: NodeJS.Platform) => boolean;

export const probeExecutable: ExecutableProbe = (candidate, platform) => {
  try {
    if (!statSync(candidate).isFile()) return false;
    if (platform !== 'win32') accessSync(candidate, constants.X_OK);
    return true;
  } catch { return false; }
};

function windowsEnvironment(env: NodeJS.ProcessEnv, name: string): string | undefined {
  const names: string[] = [];
  for (const key in env) if (key.toUpperCase() === name) names.push(key);
  return env[names.sort()[0] ?? name];
}

export function readEnvPath(env: NodeJS.ProcessEnv, platform = process.platform): string | undefined {
  return platform === 'win32' ? windowsEnvironment(env, 'PATH') : env.PATH;
}

export function windowsPathExtensions(env: NodeJS.ProcessEnv): readonly string[] {
  const fallback = ['.COM', '.EXE', '.BAT', '.CMD'];
  const parsed = (windowsEnvironment(env, 'PATHEXT') ?? '').split(';').map(entry => entry.trim()).filter(Boolean)
    .map(entry => (entry.startsWith('.') ? entry : `.${entry}`).toUpperCase());
  return parsed.length > 0 ? [...new Set(parsed)] : fallback;
}

function commandCandidates(command: string, platform: NodeJS.Platform, extensions: readonly string[]): string[] {
  if (platform !== 'win32') return [command];
  const extension = win32.extname(command).toUpperCase();
  if (extension && extensions.includes(extension)) return [command];
  return [...new Set(extensions.flatMap(ext => [`${command}${ext}`, `${command}${ext.toLowerCase()}`]))];
}

/** Finds `command` on PATH the way a spawned process would, or returns null. */
export function resolveCommandPath(command: string, platform: NodeJS.Platform,
  env: NodeJS.ProcessEnv, probe: ExecutableProbe = probeExecutable): string | null {
  const path = platform === 'win32' ? win32 : posix;
  const candidates = commandCandidates(command, platform, platform === 'win32' ? windowsPathExtensions(env) : []);
  for (const entry of (readEnvPath(env, platform) ?? '').split(platform === 'win32' ? ';' : ':')) {
    const directory = platform === 'win32' ? entry.trim().replace(/^"+|"+$/g, '') : entry;
    if (!directory || !path.isAbsolute(directory)) continue;
    for (const candidate of candidates) {
      const full = path.join(directory, candidate);
      if (probe(full, platform)) return full;
    }
  }
  return null;
}
