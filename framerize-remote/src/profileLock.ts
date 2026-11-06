import { mkdir, open, realpath, rm, type FileHandle } from 'node:fs/promises';
import { basename, dirname, join, resolve } from 'node:path';
import { ProfileLockCleanupError, RemoteDiagnosticError } from './diagnostics.js';
import { cancellableRead } from './credentials.js';

async function prepareLockPath(path: string, signal?: AbortSignal): Promise<string> {
  const directory = dirname(resolve(path));
  await mkdir(directory, { recursive: true, mode: 0o700 });
  signal?.throwIfAborted();
  const canonical = await realpath(directory);
  signal?.throwIfAborted();
  return join(canonical, `.${basename(path)}.lock`);
}

/** Hold one profile writer through its reads, writes and rollback. */
export async function withProfileLock<T>(path: string, operation: () => Promise<T>,
  signal?: AbortSignal): Promise<T> {
  signal?.throwIfAborted();
  let lockPath: string;
  let handle: FileHandle;
  try {
    lockPath = await cancellableRead(prepareLockPath(path, signal), signal);
    signal?.throwIfAborted();
  } catch {
    signal?.throwIfAborted();
    throw new RemoteDiagnosticError('profile_write');
  }
  try {
    // Drain an exclusive open before releasing ownership, even after cancellation.
    handle = await open(lockPath, 'wx', 0o600);
  } catch (error) {
    signal?.throwIfAborted();
    if ((error as NodeJS.ErrnoException).code === 'EEXIST') {
      throw new RemoteDiagnosticError('profile_busy');
    }
    throw new RemoteDiagnosticError('profile_write');
  }
  let result: { value: T } | undefined;
  let failure: unknown;
  try {
    signal?.throwIfAborted();
    try { await handle.writeFile(`${process.pid}\n`, { signal }); }
    catch {
      signal?.throwIfAborted();
      throw new RemoteDiagnosticError('profile_write');
    }
    signal?.throwIfAborted();
    result = { value: await operation() };
  } catch (error) { failure = error; }
  const failures: unknown[] = [];
  try { await handle.close(); } catch (error) { failures.push(error); }
  try { await rm(lockPath, { force: true }); } catch (error) { failures.push(error); }
  if (failures.length) throw new ProfileLockCleanupError(failure, failures);
  if (result) return result.value;
  throw failure;
}
