import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import { runInNewContext } from "node:vm";
import { CastleError } from "../dist/castle.js";
import { envelope, encodeText, MAX_ENVELOPE_BYTES } from "../dist/castle.js";
import { encodeValues } from "../dist/castle.js";
import { envelopeInputs } from "../fixtures/castle/envelope-inputs.js";
import { encodingInputs } from "../fixtures/castle/encoding-inputs.js";
const failure = (error) => error instanceof CastleError && error.code === "encoder" && error.message === "Castle Library encode-token failed";
function framed(text) {
  const bytes = Buffer.from(text);
  const length = Buffer.alloc(4);
  length.writeUInt32BE(bytes.length);
  return Buffer.concat([length, bytes]);
}
function timestampText(bytes) {
  assert.equal(bytes.length % 2, 0);
  let text = "";
  for (let index = 0; index < bytes.length; index += 2) {
    const value = bytes.readUInt16BE(index);
    const rotated = (value >>> 2 | value << 14) & 65535;
    const subtracted = rotated - 50305 & 65535;
    text += String.fromCharCode((subtracted >>> 1 | subtracted << 15) & 65535);
  }
  return text;
}
test("envelope preserves all UTF-16 code units exactly as the Rust reference", () => {
  const units = Array.from({ length: 65536 }, (_, index) => String.fromCharCode(index)).join("");
  const encoded = encodeText(units);
  assert.equal(encoded.length, 174764);
  assert.equal(
    createHash("sha256").update(encoded).digest("hex"),
    "14d69d380c8c05c11e652f7457370875948d3ee4147ea59ff5a9302741e9cc14"
  );
  assert.equal(timestampText(Buffer.from(encoded, "base64")), units);
});
test("envelope matches 181 Rust cases with complete timestamp and payload framing", () => {
  const digest = createHash("sha256");
  let total = 0;
  let count = 0;
  for (const { name, bytes, timestamp } of envelopeInputs()) {
    const snapshot = Buffer.from(bytes);
    const token = envelope(bytes, timestamp);
    assert.ok(token.startsWith("cNz8rPYH|"), name);
    const decoded = Buffer.from(token.slice(9), "base64");
    assert.equal(decoded.toString("base64"), token.slice(9), name);
    const offset = 2 + decoded.readUInt16LE();
    assert.equal(
      timestampText(Buffer.from(decoded.subarray(2, offset).toString("ascii"), "base64")),
      String(timestamp),
      name
    );
    assert.equal(decoded.length - offset, bytes.length + 24, name);
    assert.equal(decoded.readUInt32BE(offset), Math.ceil(bytes.length / 2), name);
    assert.equal(decoded.readUInt32BE(offset + 4), Math.floor(bytes.length / 2), name);
    assert.deepEqual(bytes, snapshot, name);
    const framedToken = framed(token);
    digest.update(framedToken);
    total += framedToken.length;
    count += 1;
  }
  assert.equal(count, 181);
  assert.equal(total, 2608221);
  assert.equal(digest.digest("hex"), "17a3a8ad5d2754869a32c582796cc03a4085112696cb0207ed57018b5043b0f6");
});
test("whole value-to-token pipeline matches 2048 immutable Rust cases", () => {
  const digest = createHash("sha256");
  let total = 0;
  let count = 0;
  for (const { values, timestamp } of encodingInputs()) {
    const token = framed(encodeValues(values, timestamp));
    digest.update(token);
    total += token.length;
    count += 1;
  }
  assert.equal(count, 2048);
  assert.equal(total, 284580);
  assert.equal(digest.digest("hex"), "b230f526cae52f25539255d8aa1ebe651420ad2d609d1c9a5d6da8691c6dfbf7");
});
test("envelope rejects invalid timestamps without coercion and normalizes negative zero", () => {
  const payload = Buffer.alloc(0);
  for (const timestamp of [
    null,
    void 0,
    "1",
    1n,
    {},
    NaN,
    Infinity,
    -Infinity,
    0.5,
    -8640000000000001,
    8640000000000001,
    { valueOf() {
      throw new Error("synthetic-secret");
    } }
  ]) {
    assert.throws(() => envelope(payload, timestamp), failure);
  }
  assert.equal(envelope(payload, -0), envelope(payload, 0));
});
test("envelope bounds intrinsic byte length and rejects proxies or detached storage", () => {
  assert.equal(MAX_ENVELOPE_BYTES, 524288);
  const oversized = new Uint8Array(MAX_ENVELOPE_BYTES + 1);
  Object.defineProperties(oversized, { length: { value: 0 }, byteLength: { value: 0 } });
  const detached = new Uint8Array(1);
  structuredClone(detached.buffer, { transfer: [detached.buffer] });
  const proxy = new Proxy(new Uint8Array(1), { get() {
    throw new Error("synthetic-secret");
  } });
  for (const value of [
    null,
    {},
    [],
    "ABC",
    new ArrayBuffer(1),
    new Int8Array(1),
    new Uint16Array(1),
    new DataView(new ArrayBuffer(1)),
    oversized,
    detached,
    proxy
  ]) {
    assert.throws(() => envelope(value, 0), failure);
  }
});
test("text transformation rejects unsupported and oversized inputs with fixed errors", () => {
  for (const value of [
    null,
    void 0,
    {},
    [],
    1,
    "a".repeat(65537),
    { toString() {
      throw new Error("synthetic-secret");
    } }
  ]) {
    assert.throws(() => encodeText(value), failure);
  }
  assert.equal(encodeText(""), "");
});
test("envelope preserves offset and cross-realm byte views without consulting public getters", () => {
  const storage = new Uint8Array([99, 1, 2, 3, 99]);
  const view = storage.subarray(1, 4);
  for (const key of ["length", "byteLength", "buffer", "byteOffset", Symbol.iterator]) {
    Object.defineProperty(view, key, { get() {
      throw new Error("synthetic-secret");
    } });
  }
  const expected = envelope(Buffer.from([1, 2, 3]), 0);
  assert.equal(envelope(view, 0), expected);
  assert.deepEqual(storage, new Uint8Array([99, 1, 2, 3, 99]));
  const foreign = runInNewContext("new Uint8Array([1, 2, 3])");
  assert.equal(envelope(foreign, 0), expected);
  const shared = new Uint8Array(new SharedArrayBuffer(3));
  shared.set([1, 2, 3]);
  assert.equal(envelope(shared, 0), expected);
});
test("encoder reads the live clock once after consuming the values", (context) => {
  const values = [1, "before"];
  const expected = encodeValues(values, 17e11);
  const clock = context.mock.method(Date, "now", () => {
    values[1] = "after";
    return 17e11;
  });
  assert.equal(encodeValues(values), expected);
  assert.equal(clock.mock.callCount(), 1);
  assert.equal(values[1], "after");
});
test("encoder uses an explicit timestamp without invoking the clock", (context) => {
  const clock = context.mock.method(Date, "now", () => {
    throw new Error("synthetic-secret");
  });
  assert.ok(encodeValues([], 0).startsWith("cNz8rPYH|"));
  assert.equal(clock.mock.callCount(), 0);
  assert.throws(() => encodeValues([]), failure);
  assert.equal(clock.mock.callCount(), 1);
});
test("encoder sanitizes invalid values and rejects invalid clock results", (context) => {
  const clock = context.mock.method(Date, "now", () => Infinity);
  const proxy = new Proxy([], { get() {
    throw new Error("synthetic-secret");
  } });
  assert.throws(() => encodeValues(proxy), failure);
  assert.equal(clock.mock.callCount(), 0);
  assert.throws(() => encodeValues([]), failure);
  assert.throws(() => encodeValues([], null), failure);
  assert.throws(() => encodeValues(["a".repeat(65537)], 0), failure);
  assert.throws(() => encodeValues(Array(4096).fill(null), 0), failure);
});
