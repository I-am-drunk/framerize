import {createHash} from 'node:crypto';
import {capture} from './collected_values.mjs';
import fs from 'node:fs';
import {types} from 'node:util';

export const COLLECTION_LIMIT = 1024 * 1024;
const SDK = '3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71';

// Keep configuration and all request collection waits. Replace only the
// final serialization/compression/envelope expression with a private copy.
export function collectionSource(bytes) {
  if (createHash('sha256').update(bytes).digest('hex') !== SDK)
    throw new Error('collection-source');
  return bytes.subarray(0, 274980).toString('utf8') + 'import.meta.collect(Q)'
    + bytes.subarray(281215).toString('utf8');
}

export function createCollector() {
  let used = false;
  return value => {
    if (used || types.isProxy(value) || !Array.isArray(value) || value.length !== 765)
      throw new Error('collection-boundary');
    used = true;
    const result = JSON.stringify({version: 1, values: capture(value)});
    if (Buffer.byteLength(result) > COLLECTION_LIMIT) throw new Error('collection-size');
    return result;
  };
}

export function writeCollection(fd, value) {
  let body;
  const header = Buffer.alloc(12);
  try {
    if (typeof value !== 'string' || value.length === 0) throw new Error('collection-value');
    body = Buffer.from(value);
    if (body.length > COLLECTION_LIMIT) throw new Error('collection-size');
    header.write('FCLT'); header[4] = 1; header.writeUInt32BE(body.length, 8);
    for (const bytes of [header, body]) {
      let at = 0;
      while (at < bytes.length) {
        const count = fs.writeSync(fd, bytes, at, bytes.length - at);
        if (count <= 0) throw new Error('collection-write');
        at += count;
      }
    }
  } finally {
    body?.fill(0); header.fill(0); fs.closeSync(fd);
  }
}
