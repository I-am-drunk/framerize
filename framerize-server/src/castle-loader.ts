import { createCastleMinter } from './castle.js';
import type { FramerMinter } from './framer-backend.js';

export async function createServerMinter(signal: AbortSignal): Promise<FramerMinter> {
  signal.throwIfAborted();
  const minter = createCastleMinter() as FramerMinter;
  signal.throwIfAborted();
  return minter;
}
