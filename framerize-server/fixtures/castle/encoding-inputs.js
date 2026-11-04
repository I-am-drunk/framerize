import { TIMESTAMPS } from "./envelope-inputs.js";
export function* encodingInputs() {
  const bits = Buffer.alloc(8);
  let state = 0x123456789abcdef0n;
  for (let index = 0; index < 2048; index += 1) {
    state = BigInt.asUintN(64, state * 6364136223846793005n + 1442695040888963407n);
    bits.writeBigUInt64BE(state);
    const text = String.fromCharCode(
      index,
      Number(state >> 32n & 65535n),
      Number(state >> 16n & 65535n),
      Number(state & 65535n)
    );
    yield {
      values: [bits.readDoubleBE(), text, [index % 2 === 0, null, void 0, index - 1024]],
      timestamp: TIMESTAMPS[index % TIMESTAMPS.length]
    };
  }
}
