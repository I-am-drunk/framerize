export function* compressionInputs() {
  const sizes = [
    0,
    1,
    2,
    3,
    4,
    7,
    31,
    32,
    33,
    127,
    128,
    129,
    255,
    256,
    257,
    258,
    259,
    1023,
    1024,
    1025,
    2048,
    4096,
    16384,
    65536,
    262144
  ];
  for (const size of sizes) {
    for (let pattern = 0; pattern < 4; pattern += 1) {
      const bytes = Buffer.alloc(size);
      let state = (1831565813 ^ size ^ pattern) >>> 0;
      for (let index = 0; index < size; index += 1) {
        state = Math.imul(state, 1664525) + 1013904223 >>> 0;
        bytes[index] = pattern === 0 ? 0 : pattern === 1 ? 65 + index % 7 : pattern === 2 ? index & 255 : state >>> 24;
      }
      yield { name: `${pattern}/${size}`, bytes };
    }
  }
}
