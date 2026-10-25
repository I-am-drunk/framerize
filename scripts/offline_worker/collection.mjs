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
  let source = bytes.subarray(0, 274980).toString('utf8') + 'import.meta.collect(Q)'
    + bytes.subarray(281215).toString('utf8');
  const assignment = 'Q[_o]=l(s,Qon(W))';
  if (source.indexOf(assignment) < 0 || source.indexOf(assignment) !== source.lastIndexOf(assignment))
    throw new Error('collection-worker-source');
  // Slot 242 has one writer and no reader before serialization. Capture at
  // that boundary; the parent reduces this plain-data result into the slot.
  source = source.replace(assignment, 'import.meta.sharedWorker(W)');
  const network = 'Q[B1]=O(Ar,T0n(W)),Q[_C]=l(k4,Brn(W)),'
    + 'Q[473]=a(X0,krn(W)),Q[Rp]=a(u0,a$(W))';
  if (source.indexOf(network) < 0 || source.indexOf(network) !== source.lastIndexOf(network))
    throw new Error('collection-network-source');
  // Keep SX and its later timing conversion. Only the four field reductions
  // move to Rust; their source destinations have no intervening reader.
  return source.replace(network, 'import.meta.network(W)');
}

export function createCollector() {
  let used = false, observed = false, reply, networkObserved = false, networkReply;
  const state = {seen: new Set(), count: 0, units: 0};
  const sharedWorker = value => {
    if (used || observed) throw new Error('collection-worker-boundary');
    observed = true;
    reply = capture(value, state);
    return value;
  };
  const network = value => {
    if (used || networkObserved) throw new Error('collection-network-boundary');
    networkObserved = true;
    networkReply = capture(value, state);
    return value;
  };
  const collect = value => {
    if (used || types.isProxy(value) || !Array.isArray(value) || value.length !== 765)
      throw new Error('collection-boundary');
    used = true;
    if (reply === undefined) throw new Error('collection-worker-missing');
    if (networkReply === undefined) throw new Error('collection-network-missing');
    const result = JSON.stringify({version: 3, values: capture(value, state),
      shared_worker_reply: reply, network_reply: networkReply});
    if (Buffer.byteLength(result) > COLLECTION_LIMIT) throw new Error('collection-size');
    return result;
  };
  return {collect, sharedWorker, network};
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
