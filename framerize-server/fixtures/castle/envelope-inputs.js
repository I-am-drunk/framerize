import { compressionInputs } from "./compression-inputs.js";
export const TIMESTAMPS = [
  -864e13,
  -2147483649,
  -1,
  0,
  1,
  2147483648,
  17e11,
  864e13
];
export function* envelopeInputs() {
  let index = 0;
  for (const input of compressionInputs()) {
    yield { ...input, timestamp: TIMESTAMPS[index++ % TIMESTAMPS.length] };
  }
  for (const timestamp of TIMESTAMPS) {
    for (let size = 0; size < 10; size += 1) {
      const bytes = Buffer.from(Array.from({ length: size }, (_, position) => position * 29));
      yield { name: `${timestamp}/${size}`, bytes, timestamp };
    }
  }
  yield { name: "maximum", bytes: Buffer.alloc(512 * 1024, 165), timestamp: 17e11 };
}
