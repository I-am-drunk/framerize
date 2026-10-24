import test from 'node:test';
import assert from 'node:assert/strict';
import { inspectEnvelope } from './envelope.mjs';

function fixture(clock = 36, payload = 24) {
  const bytes = Buffer.alloc(2 + clock + payload, 0x41);
  bytes.writeUInt16LE(clock);
  return 'cNz8rPYH|' + bytes.toString('base64');
}

test('reports lengths without retaining clock or payload contents', () => {
  assert.deepEqual(inspectEnvelope(fixture()), {
    format_matches: true, decoded_bytes: 62, clock_bytes: 36, payload_bytes: 24,
  });
});
test('rejects the SDK prefix-only overflow result', () => {
  assert.throws(() => inspectEnvelope('cNz8rPYH|'), { message: 'envelope-shape' });
});
test('rejects truncated inner payload and impossible clock lengths', () => {
  for (const value of [fixture(36, 23), fixture(0), fixture().slice(0, -4)])
    assert.throws(() => inspectEnvelope(value), { message: 'envelope-shape' });
});
test('rejects wrong prefix, invalid base64 and oversized values', () => {
  for (const value of [fixture().replace('cNz8', 'xxxx'), fixture() + '!',
    fixture() + '\n', 'cNz8rPYH|' + 'A'.repeat(16384)])
    assert.throws(() => inspectEnvelope(value), { message: 'envelope-shape' });
});
