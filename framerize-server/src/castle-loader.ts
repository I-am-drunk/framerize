import { rm } from 'node:fs/promises';
import { join } from 'node:path';
import { getAsset, isSea } from 'node:sea';
import { pathToFileURL } from 'node:url';
import { materializeRuntime } from './castle-assets.js';
import { ConfigurationError } from './errors.js';
import type { FramerMinter } from './framer-backend.js';

export async function createServerMinter(signal: AbortSignal): Promise<FramerMinter> {
  let directory: string | undefined;
  let minter: FramerMinter | undefined;
  try {
    signal.throwIfAborted();
    if (isSea()) directory = await materializeRuntime(getAsset('castle-runtime'), signal);
    const url = directory ? pathToFileURL(join(directory, 'dist/castle.js'))
      : new URL('./castle/dist/castle.js', import.meta.url || pathToFileURL(__filename).href);
    const library = await import(url.href) as { createCastleMinter(): FramerMinter };
    minter = library.createCastleMinter();
    signal.throwIfAborted();
    return ownedMinter(minter, directory);
  } catch {
    try { await minter?.close(); }
    finally { if (directory) await rm(directory, { recursive: true, force: true }); }
    throw new ConfigurationError('Castle runtime unavailable');
  }
}

function ownedMinter(minter: FramerMinter, directory: string | undefined): FramerMinter {
  let closing: Promise<void> | undefined;
  return Object.freeze({
    mint: (request: Parameters<FramerMinter['mint']>[0]) => minter.mint(request),
    close() {
      closing ??= Promise.resolve().then(async () => {
        try { await minter.close(); }
        finally { if (directory) await rm(directory, { recursive: true, force: true }); }
      });
      return closing;
    },
  });
}
