import fs from 'node:fs';
import { createHash } from 'node:crypto';
import { performance } from 'node:perf_hooks';
import { pathToFileURL } from 'node:url';
import { readInput, verifyDescriptors } from './protocol.mjs';
import { createLegacyV2Minter, verifyV2Inputs } from './v2_runtime.mjs';
import { runV2Handoff } from './v2_handoff.mjs';
import { SUPPORT } from './pins.mjs';

const exit = await runV2Handoff({
  verifyDescriptors, readInput, createMinter: createLegacyV2Minter,
  now: () => performance.now(),
  verify() {
    verifyV2Inputs();
    const [path, hash] = SUPPORT.writer;
    if (createHash('sha256').update(fs.readFileSync(path)).digest('hex') !== hash)
      throw new Error('writer-identity');
  },
  async writer() {
    const { makeTokenWriter } = await import(pathToFileURL(SUPPORT.writer[0]));
    return makeTokenWriter(4);
  },
  closeToken() { fs.closeSync(4); },
  writeMetadata(report) {
    const bytes = Buffer.from(JSON.stringify(report) + '\n');
    if (bytes.length > 16384) throw new Error('metadata-size');
    let offset = 0;
    while (offset < bytes.length) {
      const count = fs.writeSync(5, bytes, offset, bytes.length - offset);
      if (count <= 0) throw new Error('metadata-write');
      offset += count;
    }
    fs.closeSync(5);
  },
});
process.exit(exit);
