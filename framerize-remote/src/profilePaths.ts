import { homedir } from 'node:os';
import { isAbsolute, join, resolve } from 'node:path';

export function profilePath(explicit?: string, env: NodeJS.ProcessEnv = process.env,
  cwd = process.cwd(), home = homedir()): string {
  if (explicit !== undefined) return resolve(cwd, explicit);
  const configured = env.FRAMERIZE_REMOTE_HOME;
  if (configured !== undefined) {
    if (!configured || !isAbsolute(configured) || /[\x00-\x1f\x7f]/.test(configured)) {
      throw new Error('FRAMERIZE_REMOTE_HOME must be an absolute directory path');
    }
    return join(configured, 'framerize-remote.json');
  }
  return join(home, '.framerize-remote', 'framerize-remote.json');
}
