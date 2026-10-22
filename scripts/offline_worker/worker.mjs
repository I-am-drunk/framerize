import fs from 'node:fs';
import { performance } from 'node:perf_hooks';
import { pathToFileURL } from 'node:url';
import { OfflineCastleMinter } from './offline_mint.mjs';
import { readInput, verifyDescriptors } from './protocol.mjs';
import { verifyInputs } from './sdk_loader.mjs';
import { SUPPORT } from './pins.mjs';

let trusted = false, tokenClosed = false, phase = 'descriptors';
let metadata = { version: 1, status: 'failed', failure_phase: phase };
try {
  verifyDescriptors(); trusted = true;
  phase = 'input';
  const minter = new OfflineCastleMinter(readInput());
  phase = 'mint';
  const result = await minter.mint();
  metadata = result.metadata;
  if (result.token) {
    phase = 'handoff';
    verifyInputs();
    const { makeTokenWriter } = await import(pathToFileURL(SUPPORT.writer[0]));
    metadata.mint_to_handoff_ms = performance.now() - result.minted_at;
    if (!Number.isFinite(metadata.mint_to_handoff_ms)
        || metadata.mint_to_handoff_ms < 0 || metadata.mint_to_handoff_ms > 2000)
      throw new Error('token-age');
    try { makeTokenWriter(4)(result.token.take()); }
    finally { tokenClosed = true; }
    metadata.mint_to_handoff_ms = performance.now() - result.minted_at;
    if (metadata.mint_to_handoff_ms > 2000) throw new Error('token-age');
    metadata.token_handoff = true;
  }
} catch {
  metadata = { version: 1, status: 'failed', failure_phase: phase };
} finally {
  if (!tokenClosed) { try { fs.closeSync(4); } catch {} }
}
if (trusted) {
  try {
    const bytes = Buffer.from(JSON.stringify(metadata) + '\n');
    if (bytes.length > 16384) throw new Error('metadata-size');
    let offset = 0;
    while (offset < bytes.length) {
      const count = fs.writeSync(5, bytes, offset, bytes.length - offset);
      if (count <= 0) throw new Error('metadata-write');
      offset += count;
    }
    fs.closeSync(5);
  } catch { process.exit(3); }
}
process.exit(metadata.status === 'passed' && metadata.token_handoff === true ? 0 : 2);
