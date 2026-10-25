import fs from 'node:fs';
import { performance } from 'node:perf_hooks';
import { OfflineCastleMinter } from './offline_mint.mjs';
import { readInput, verifyDescriptors } from './protocol.mjs';
import { verifyInputs } from './sdk_loader.mjs';
import { writeCollection } from './collection.mjs';

let trusted = false, tokenClosed = false, phase = 'descriptors';
let pipeWriteAttempted = false, pipeWriteCompleted = false;
let handoffAge = null;
let metadata = { version: 1, status: 'failed', failure_phase: phase };
try {
  verifyDescriptors(); trusted = true;
  phase = 'input';
  const minter = new OfflineCastleMinter(readInput());
  phase = 'mint';
  const result = await minter.mint();
  metadata = result.metadata;
  if (result.collection) {
    phase = 'handoff';
    verifyInputs();
    metadata.collection_to_handoff_ms = handoffAge = performance.now() - result.collected_at;
    if (!Number.isFinite(handoffAge) || handoffAge < 0 || handoffAge > 2000)
      throw new Error('collection-age');
    const collection = result.collection.take();
    pipeWriteAttempted = true;
    try { writeCollection(4, collection); pipeWriteCompleted = true; }
    finally { tokenClosed = true; }
    metadata.collection_to_handoff_ms = handoffAge = performance.now() - result.collected_at;
    if (!Number.isFinite(handoffAge) || handoffAge < 0 || handoffAge > 2000)
      throw new Error('collection-age');
    metadata.collection_handoff = true;
  }
} catch {
  if (phase === 'handoff') {
    // The minter's completed facts survive a later refusal. A returned pipe
    // writer does not establish freshness, parent receipt or joined cleanup.
    metadata.status = 'failed';
    metadata.failure_phase = phase;
    metadata.collection_handoff = false;
    metadata.collection_pipe_write_attempted = pipeWriteAttempted;
    metadata.collection_pipe_write_completed = pipeWriteCompleted;
    metadata.collection_to_handoff_ms = Number.isFinite(handoffAge) && handoffAge >= 0
      ? handoffAge : null;
  } else {
    metadata = { version: 1, status: 'failed', failure_phase: phase };
  }
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
process.exit(metadata.status === 'passed' && metadata.collection_handoff === true ? 0 : 2);
