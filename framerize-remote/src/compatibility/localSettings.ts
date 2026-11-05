import { lstat } from 'node:fs/promises';
import { isAbsolute, join } from 'node:path';
import { defaultRealPath, type ClientSetupDeps } from './deps.js';

const fileOwner = async (path: string): Promise<number | null> =>
  lstat(path).then(stat => stat.uid, () => null);

async function captureRoot(deps: ClientSetupDeps, cwd: string): Promise<string | null> {
  const controller = new AbortController(), signal = deps.signal ?? controller.signal;
  const stop = (): void => controller.abort();
  if (!deps.signal) { process.on('SIGINT', stop); process.on('SIGTERM', stop); }
  try {
    const output = await deps.runner.capture({ command: 'git',
      args: ['-C', cwd, 'rev-parse', '--show-toplevel'], display: 'git rev-parse --show-toplevel' },
    deps.env, signal);
    signal.throwIfAborted();
    return output;
  } finally {
    if (!deps.signal) { process.off('SIGINT', stop); process.off('SIGTERM', stop); }
  }
}

/** Claude moves local settings to an owned Git root on POSIX, except for a home-directory repo. */
export async function claudeLocalSettingsPath(deps: ClientSetupDeps, cwd: string): Promise<string> {
  const local = join(cwd, '.claude', 'settings.local.json');
  deps.signal?.throwIfAborted();
  const uid = process.getuid?.();
  if (deps.platform === 'win32' || uid === undefined) return local;
  const output = await captureRoot(deps, cwd);
  deps.signal?.throwIfAborted();
  const root = output?.replace(/\n$/, '');
  if (!root || !isAbsolute(root)) return local;
  const realPath = deps.realPath ?? defaultRealPath;
  if (realPath(root) === realPath(deps.home)) return local;
  const owner = deps.fileOwner ?? fileOwner;
  const [directory, git, settings] = await Promise.all([
    owner(root), owner(join(root, '.git')), owner(join(root, '.claude')),
  ]);
  deps.signal?.throwIfAborted();
  return directory === uid && git === uid && (settings === null || settings === uid)
    ? join(root, '.claude', 'settings.local.json') : local;
}
