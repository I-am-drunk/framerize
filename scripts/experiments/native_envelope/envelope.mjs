// Structural evidence for pinned SDK 3337eba7, never token authentication.
// No token contents or decoded fields leave this function.
export function inspectEnvelope(value) {
  const reject = () => { throw new Error('envelope-shape'); };
  if (typeof value !== 'string' || value.length > 16384) reject();
  const prefix = 'cNz8rPYH|';
  if (!value.startsWith(prefix)) reject();
  const encoded = value.slice(prefix.length);
  if (!encoded || encoded.length % 4 !== 0
      || !/^[A-Za-z0-9+/]+={0,2}$/.test(encoded)) reject();
  const bytes = Buffer.from(encoded, 'base64');
  try {
    if (bytes.toString('base64') !== encoded || bytes.length < 26) reject();
    const clockBytes = bytes.readUInt16LE(0);
    const payloadBytes = bytes.length - 2 - clockBytes;
    if (clockBytes === 0 || payloadBytes < 24) reject();
    return { format_matches: true, decoded_bytes: bytes.length,
      clock_bytes: clockBytes, payload_bytes: payloadBytes };
  } finally {
    bytes.fill(0);
  }
}
