import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import { runInNewContext } from "node:vm";
import { inflateRawSync } from "node:zlib";
import { CastleError } from "../dist/castle.js";
import { compress, MAX_COMPRESSION_BYTES } from "../dist/castle.js";
import { huffmanCodes, huffmanLengths } from "../dist/castle.js";
import { tokenize } from "../dist/castle.js";
import { compressionInputs } from "../fixtures/castle/compression-inputs.js";
function refuses(input) {
  assert.throws(() => compress(input), (error) => error instanceof CastleError && error.code === "encoder" && error.message === "Castle Library encode-token failed");
}
test("compressor matches 100 immutable Rust samples and preserves their input bytes", () => {
  const digest = createHash("sha256");
  let total = 0;
  let count = 0;
  for (const { name, bytes } of compressionInputs()) {
    const snapshot = Buffer.from(bytes);
    const compressed = compress(bytes);
    assert.equal(compressed[0] & 7, 5, name);
    assert.deepEqual(inflateRawSync(compressed), snapshot, name);
    assert.deepEqual(bytes, snapshot, name);
    const length = Buffer.alloc(4);
    length.writeUInt32BE(compressed.length);
    digest.update(length).update(compressed);
    total += 4 + compressed.length;
    count += 1;
  }
  assert.equal(count, 100);
  assert.equal(total, 366556);
  assert.equal(digest.digest("hex"), "ca6388f57e7c19a816a97f193e547c6343f21e814c97ba6885b5bfad9c199e9e");
});
test("compressor accepts byte views without adjacent storage or public getter access", () => {
  const storage = new Uint8Array([90, 65, 66, 67, 89]);
  const view = storage.subarray(1, 4);
  for (const name of ["length", "byteLength", "buffer", "byteOffset", Symbol.iterator]) {
    Object.defineProperty(view, name, { get() {
      throw new Error("synthetic-secret");
    } });
  }
  assert.deepEqual(inflateRawSync(compress(view)), Buffer.from("ABC"));
  assert.deepEqual(storage, new Uint8Array([90, 65, 66, 67, 89]));
  const foreign = runInNewContext("new Uint8Array([65, 66, 67])");
  assert.deepEqual(compress(foreign), compress(view));
});
test("compressor rejects oversized inputs despite overridden length getters", () => {
  assert.equal(MAX_COMPRESSION_BYTES, 262144);
  const oversized = new Uint8Array(MAX_COMPRESSION_BYTES + 1);
  refuses(oversized);
  Object.defineProperties(oversized, { byteLength: { value: 0 }, length: { value: 0 } });
  refuses(oversized);
});
test("compressor rejects non-byte inputs, detached storage and proxies with fixed errors", () => {
  for (const value of [
    null,
    void 0,
    {},
    [],
    "ABC",
    12,
    new ArrayBuffer(3),
    new DataView(new ArrayBuffer(3)),
    new Int8Array(3),
    new Uint16Array(3)
  ]) refuses(value);
  const input = new Uint8Array(3);
  refuses(new Proxy(input, { get() {
    throw new Error("synthetic-secret");
  } }));
  const revoked = Proxy.revocable(input, {});
  revoked.revoke();
  refuses(revoked.proxy);
  structuredClone(input.buffer, { transfer: [input.buffer] });
  refuses(input);
});
test("compressor snapshots resizable and shared byte storage", () => {
  const resizable = Reflect.construct(ArrayBuffer, [3, { maxByteLength: 10 }]);
  const shared = new SharedArrayBuffer(3);
  for (const storage of [resizable, shared]) {
    const input = new Uint8Array(storage);
    input.set([65, 66, 67]);
    const compressed = compress(input);
    input.fill(0);
    assert.deepEqual(inflateRawSync(compressed), Buffer.from("ABC"));
  }
});
test("Huffman construction preserves empty, singleton and tied alphabets", () => {
  assert.deepEqual(huffmanLengths([0, 0, 0], 15), [0, 0, 0]);
  assert.deepEqual(huffmanLengths([0, 9, 0], 15), [0, 1, 0]);
  assert.deepEqual(huffmanLengths([1, 1, 1], 15), [2, 2, 1]);
  assert.deepEqual(huffmanLengths([1, 1, 1, 1], 15), [2, 2, 2, 2]);
  assert.deepEqual(huffmanCodes([2, 2, 1, 0], 15), [2, 3, 0, 0]);
});
test("Huffman construction enforces both code-length limits without oversubscription", () => {
  for (const maximum of [7, 15]) {
    const weights = Array.from({ length: 19 }, (_, index) => 2 ** index);
    const lengths = huffmanLengths(weights, maximum);
    assert.equal(Math.max(...lengths), maximum);
    assert.ok(lengths.every((length) => length > 0 && length <= maximum));
    assert.equal(lengths.reduce((sum, length) => sum + 2 ** (maximum - length), 0), 2 ** maximum);
    const words = huffmanCodes(lengths, maximum).map((code, index) => code.toString(2).padStart(lengths[index], "0"));
    for (const [index, word] of words.entries()) {
      assert.ok(words.every((other, position) => position === index || !other.startsWith(word)));
    }
  }
});
test("matcher supports overlapping copies and limits each match to 258 bytes", () => {
  assert.deepEqual(tokenize(Buffer.from("abcabcabc")), [97, 98, 99, { length: 6, distance: 3 }]);
  assert.deepEqual(tokenize(Buffer.alloc(260, 65)), [65, { length: 258, distance: 1 }, 65]);
});
test("matcher keeps the 1024-byte window inclusive", () => {
  const prefix = Buffer.from("ABC");
  const within = Buffer.concat([prefix, Buffer.alloc(1021), prefix]);
  const outside = Buffer.concat([prefix, Buffer.alloc(1022), prefix]);
  assert.deepEqual(tokenize(within).at(-1), { length: 3, distance: 1024 });
  assert.deepEqual(tokenize(outside).slice(-3), [65, 66, 67]);
});
test("matcher searches at most 64 candidates and keeps the closest tied match", () => {
  const input = (fillers) => Buffer.concat([
    Buffer.from("ABCD"),
    Buffer.from("ABCX".repeat(fillers)),
    Buffer.from([255]),
    Buffer.from("ABCD")
  ]);
  assert.deepEqual(tokenize(input(63)).at(-1), { length: 4, distance: 257 });
  assert.deepEqual(tokenize(input(64)).slice(-2), [{ length: 3, distance: 5 }, 68]);
});
