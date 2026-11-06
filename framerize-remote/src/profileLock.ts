import { mkdir, open, realpath, rm, type FileHandle } from 'node:fs/promises';
import { basename, dirname, join, resolve } from 'node:path';
import { RemoteDiagnosticError } from './diagnostics.js';

/** Hold one profile writer through its reads, writes and rollback. */
export async function withProfileLock<T>(path: string, operation: () => Promise<T>): Promise<T> {
  let lockPath: string;
  let handle: FileHandle;
  try {
    const directory = dirname(resolve(path));
    await mkdir(directory, { recursive: true, mode: 0o700 });
    lockPath = join(await realpath(directory), `.${basename(path)}.lock`);
  } catch {
    throw new RemoteDiagnosticError('profile_write');
  }
  try {
    handle = await open(lockPath, 'wx', 0o600);
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'EEXIST') {
      throw new RemoteDiagnosticError('profile_busy');
    }
    throw new RemoteDiagnosticError('profile_write');
  }
  try {
    try { await handle.writeFile(`${process.pid}\n`); }
    catch { throw new RemoteDiagnosticError('profile_write'); }
    return await operation();
  } finally {
    try { await handle.close(); }
    finally { await rm(lockPath, { force: true }); }
  }
}
