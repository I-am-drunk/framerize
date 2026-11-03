// Source only. The live wrapper variant must finish its integrity/cleanup
// checks before calling this function. No existing wrapper file imports it.
import fs from 'node:fs';

const MAX_TOKEN_BYTES = 16 * 1024;
const MAGIC = Buffer.from([0x46, 0x53, 0x44, 0x4b, 1, 0, 0, 0]);

export function makeTokenWriter(descriptor) {
  if (!Number.isInteger(descriptor) || descriptor < 3 || descriptor > 1024 ||
      !fs.fstatSync(descriptor).isFIFO()) throw new Error('handoff-descriptor');
  const info = fs.readFileSync('/proc/self/fdinfo/' + descriptor, 'utf8');
  const flags = /^flags:\s+([0-7]+)$/m.exec(info);
  if (!flags || (Number.parseInt(flags[1], 8) & 3) !== 1)
    throw new Error('handoff-direction');
  let used = false;
  return value => {
    if (used) throw new Error('handoff-already-used');
    used = true;
    let bytes;
    try {
      if (typeof value !== 'string' || value.length === 0 ||
          value.length > MAX_TOKEN_BYTES) throw new Error('handoff-type');
      bytes = Buffer.from(value, 'utf8');
      if (bytes.length > MAX_TOKEN_BYTES) throw new Error('handoff-size');
      if (bytes.toString('utf8') !== value) throw new Error('handoff-encoding');
      const header = Buffer.alloc(12);
      MAGIC.copy(header);
      header.writeUInt32BE(bytes.length, 8);
      for (const buffer of [header, bytes]) {
        let written = 0;
        while (written < buffer.length) {
          const count = fs.writeSync(descriptor, buffer, written, buffer.length - written);
          if (count <= 0) throw new Error('handoff-write');
          written += count;
        }
      }
    } catch {
      throw new Error('handoff-failed');
    } finally {
      bytes?.fill(0);
      fs.closeSync(descriptor);
    }
  };
}
