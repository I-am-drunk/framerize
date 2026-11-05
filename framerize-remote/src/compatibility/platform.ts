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

export function readEnvPath(env: NodeJS.ProcessEnv): string | undefined {
  return env.PATH ?? env.Path ?? env.path;
}

export function windowsPathExtensions(env: NodeJS.ProcessEnv): readonly string[] {
  const fallback = ['.COM', '.EXE', '.BAT', '.CMD'];
  const parsed = (env.PATHEXT ?? '').split(';').map(entry => entry.trim()).filter(Boolean)
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
  const candidates = commandCandidates(command, platform, windowsPathExtensions(env));
  for (const entry of (readEnvPath(env) ?? '').split(platform === 'win32' ? ';' : ':')) {
    const directory = entry.trim().replace(/^"+|"+$/g, '');
    if (!directory || !path.isAbsolute(directory)) continue;
    for (const candidate of candidates) {
      const full = path.join(directory, candidate);
      if (probe(full, platform)) return full;
    }
  }
  return null;
}
