// Test-only observation hooks. The production SDK loader does not import this.
import {createHash} from 'node:crypto';
const sdkHash = '3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71';
const sha = value => createHash('sha256').update(value).digest('hex');
let serialized, timestamp, instrumentedHash;

function once(source, before, after) {
  const at = source.indexOf(before);
  if (at < 0 || source.indexOf(before, at + before.length) >= 0)
    throw new Error('snapshot-source-boundary');
  return source.slice(0, at) + after + source.slice(at + before.length);
}

export function instrument(role, bytes) {
  if (role !== 'v3') return bytes.toString('utf8');
  if (sha(bytes) !== sdkHash || instrumentedHash) throw new Error('snapshot-sdk-identity');
  let body = bytes.subarray(274980, 281222).toString('utf8');
  body = once(body, '$2n((function(L){', '$2n(import.meta.serialized((function(L){');
  body = once(body, '})(Q))[En](function(L){', '})(Q)))[En](function(L){');
  body = once(body, 'a(jf)[Tn]()', 'import.meta.clock(a(jf))[Tn]()');
  const source = bytes.subarray(0, 274980).toString('utf8') + body + bytes.subarray(281222).toString('utf8');
  instrumentedHash = sha(source);
  return source;
}

export function initializeRecorder(role, meta) {
  if (role !== 'v3') return;
  meta.serialized = value => {
    if (serialized || !ArrayBuffer.isView(value) || value.BYTES_PER_ELEMENT !== 1
        || value.byteLength > 1024 * 1024) throw new Error('snapshot-bytes');
    serialized = Buffer.from(value); return value;
  };
  meta.clock = value => {
    if (timestamp !== undefined || !Number.isSafeInteger(value)) throw new Error('snapshot-clock');
    timestamp = value; return value;
  };
}

export function takeSnapshot(token) {
  if (!serialized || timestamp === undefined || !instrumentedHash || typeof token !== 'string')
    throw new Error('snapshot-incomplete');
  const result = {schema:1, synthetic:true, sdk_sha256:sdkHash,
    instrumented_sdk_sha256:instrumentedHash, timestamp_ms:timestamp,
    serialized:Array.from(serialized), token_sha256:sha(token), token_bytes:Buffer.byteLength(token)};
  serialized.fill(0); serialized = undefined;
  return result;
}
