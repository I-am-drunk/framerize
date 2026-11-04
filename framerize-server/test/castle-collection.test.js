import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import { CastleError } from "../dist/castle.js";
import { decodeCollection } from "../dist/castle.js";
import { encodeFrame } from "../dist/castle.js";
import { encodeText } from "../dist/castle.js";
import { capture, collectionInput, collectionInputs, frame } from "../fixtures/castle/collection-inputs.js";
const failure = (error) => error instanceof CastleError && error.code === "encoder" && error.message === "Castle Library encode-token failed";
const decode = (input) => decodeCollection(JSON.stringify(input));
test("decoder preserves typed values and applies only the five collector assignments", () => {
  const values = Array(765).fill(void 0);
  values[0] = [void 0, null, -0, NaN, Infinity, -Infinity, "\uD800", true];
  values[755] = "sdk-owned-timing";
  const decoded = decode(collectionInput(values));
  const expected = [...values];
  expected[242] = "a9opjk";
  expected[473] = "a9opjk";
  for (const position of [80, 469, 659]) expected[position] = encodeText("a9opjk");
  assert.deepEqual(decoded, expected);
});
test("complete captured frames match 128 immutable Rust mapping and encoding cases", () => {
  const digest = createHash("sha256");
  let count = 0;
  let bytes = 0;
  for (const { text, timestamp } of collectionInputs()) {
    const input = frame(text);
    const snapshot = Buffer.from(input);
    const token = Buffer.from(encodeFrame(input, timestamp));
    const length = Buffer.alloc(4);
    length.writeUInt32BE(token.length);
    digest.update(length).update(token);
    bytes += token.length + 4;
    count += 1;
    assert.deepEqual(input, snapshot);
  }
  assert.equal(count, 128);
  assert.equal(bytes, 34520);
  assert.equal(digest.digest("hex"), "13846785e315241b3cec3152555acab063540942b370ae41551afa691b4eaab4");
});
test("decoder requires exact complete schema and both observed replies", () => {
  const valid = collectionInput();
  for (const input of [
    null,
    {},
    [],
    { ...valid, version: 2 },
    { ...valid, extra: true },
    { ...valid, shared_worker_reply: void 0 },
    { ...valid, network_reply: void 0 },
    { ...valid, shared_worker_reply: null },
    { ...valid, network_reply: null }
  ]) {
    assert.throws(() => decode(input), failure);
  }
  for (const size of [0, 764, 766]) {
    assert.throws(() => decode(collectionInput(Array(size).fill(void 0))), failure);
  }
});
test("decoder rejects malformed captured values and noncanonical numbers", () => {
  const bad = [
    { kind: "object" },
    { kind: "null", raw: "synthetic-secret" },
    { kind: "boolean", value: 1 },
    { kind: "string", units: [65536] },
    { kind: "string", units: [-1] },
    { kind: "string", units: [1.5] },
    { kind: "string", units: ["1"] },
    { kind: "array", values: Array(2049).fill(capture(null)) }
  ];
  for (const value of ["inf", "01", "1e999", "1.0", "+1", " 1", "", "-NaN"]) {
    bad.push({ kind: "number", value });
  }
  for (const value of bad) {
    const input = collectionInput();
    input.values.values[0] = value;
    assert.throws(() => decode(input), failure);
  }
});
test("decoder rejects duplicate keys including escaped spellings", () => {
  const text = JSON.stringify(collectionInput());
  for (const replacement of ['"version":3,"version":3', '"version":3,"\\u0076ersion":3']) {
    assert.throws(() => decodeCollection(text.replace('"version":3', replacement)), failure);
  }
  assert.throws(() => decodeCollection(text.replace(
    '"kind":"undefined"',
    '"kind":"null","kind":"undefined"'
  )), failure);
  const input = collectionInput();
  input.values.values[0] = capture('quoted " }, [, \\ text');
  assert.equal(decode(input)[0], 'quoted " }, [, \\ text');
  assert.equal(decodeCollection(JSON.stringify(input, null, 2))[0], decode(input)[0]);
});
test("decoder bounds combined count, depth and UTF-16 units", () => {
  const values = Array(765).fill(void 0);
  values[0] = Array(1664).fill(null);
  values[1] = Array(1664).fill(null);
  assert.equal(decode(collectionInput(values)).length, 765);
  values[1] = Array(1665).fill(null);
  assert.throws(() => decode(collectionInput(values)), failure);
  values[1] = void 0;
  values[0] = "\uD800".repeat(65536);
  assert.equal(decode(collectionInput(values))[0].length, 65536);
  assert.throws(() => decode(collectionInput(values, "x")), failure);
  values[0] = null;
  for (let depth = 0; depth < 63; depth += 1) values[0] = [values[0]];
  assert.equal(decode(collectionInput(values)).length, 765);
  values[0] = [values[0]];
  assert.throws(() => decode(collectionInput(values)), failure);
});
test("network reduction refuses absent records and non-string transformed fields", () => {
  for (const network of [[], [null], [void 0]]) {
    assert.throws(() => decode(collectionInput(void 0, void 0, network)), failure);
  }
  for (const field of [40, 47, 18]) {
    const record = Array(48).fill(void 0);
    record[field] = true;
    assert.throws(() => decode(collectionInput(void 0, void 0, [record])), failure);
  }
  const decoded = decode(collectionInput(void 0, void 0, ["a".repeat(48)]));
  assert.equal(decoded[473], "a");
  assert.equal(decoded[80], encodeText("a"));
});
test("frame decoder rejects malformed headers, length, UTF-8 and JSON", () => {
  const valid = frame(JSON.stringify(collectionInput()));
  for (const position of [0, 4, 5, 6, 7, 11]) {
    const input = Buffer.from(valid);
    input[position] = input[position] ^ 1;
    assert.throws(() => encodeFrame(input, 0), failure);
  }
  for (const input of [
    valid.subarray(0, -1),
    Buffer.concat([valid, Buffer.from([0])]),
    frame("\uFEFF" + valid.subarray(12).toString()),
    frame("{"),
    Buffer.alloc(1024 * 1024 + 13)
  ]) {
    assert.throws(() => encodeFrame(input, 0), failure);
  }
  const invalid = frame(" ");
  invalid[12] = 255;
  assert.throws(() => encodeFrame(invalid, 0), failure);
});
test("frame encoder enforces the final token limit", () => {
  let text = "";
  let state = 123456789;
  for (let index = 0; index < 1e4; index += 1) {
    state = Math.imul(state, 1664525) + 1013904223 >>> 0;
    text += String.fromCharCode(state >>> 16);
  }
  const values = Array(765).fill(void 0);
  values[0] = text;
  assert.throws(() => encodeFrame(frame(JSON.stringify(collectionInput(values))), 0), failure);
});
test("decoder rejects active inputs and excessive text with fixed errors", () => {
  const proxy = new Proxy({}, { get() {
    throw new Error("synthetic-secret");
  } });
  for (const value of [null, {}, proxy, "", " ".repeat(1024 * 1024 + 1)]) {
    assert.throws(() => decodeCollection(value), failure);
  }
  for (const value of [null, {}, proxy, new Uint16Array(20)]) {
    assert.throws(() => encodeFrame(value, 0), failure);
  }
  const oversized = new Uint8Array(1024 * 1024 + 13);
  Object.defineProperties(oversized, { length: { value: 12 }, byteLength: { value: 12 } });
  assert.throws(() => encodeFrame(oversized, 0), failure);
  assert.throws(() => decodeCollection("[".repeat(257) + "]".repeat(257)), failure);
});
