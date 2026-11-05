import { homedir } from 'node:os';
import { dirname, isAbsolute, join, resolve } from 'node:path';
import { RemoteDiagnosticError } from './diagnostics.js';

export function profilePath(explicit?: string, env: NodeJS.ProcessEnv = process.env,
  cwd = process.cwd(), home = homedir()): string {
  if (explicit !== undefined) return resolve(cwd, explicit);
  const configured = env.FRM_HOME ?? env.FRAMERIZE_REMOTE_HOME;
  if (configured !== undefined) {
    if (!configured || !isAbsolute(configured) || /[\x00-\x1f\x7f]/.test(configured)) {
      throw new RemoteDiagnosticError('profile_home_invalid');
    }
    return join(configured, 'framerize-remote.json');
  }
  return join(home, '.framerize-remote', 'framerize-remote.json');
}

export function profileHome(env: NodeJS.ProcessEnv = process.env, home = homedir()): string {
  return dirname(profilePath(undefined, env, process.cwd(), home));
}
