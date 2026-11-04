import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import test from "node:test";
import { CastleError } from "../dist/castle.js";
import { serializeValues } from "../dist/castle.js";
const cases = [
  ["empty array", [], "00"],
  ["null", [null], "0100"],
  ["undefined", [void 0], "0100"],
  ["false", [false], "0105"],
  ["true", [true], "0106"],
  ["NaN", [NaN], "0100"],
  ["positive infinity", [Infinity], "0100"],
  ["negative infinity", [-Infinity], "0100"],
  ["zero", [0], "010a00"],
  ["negative zero", [-0], "010a00"],
  ["one", [1], "010a02"],
  ["negative one", [-1], "010a01"],
  ["signed maximum", [2147483647], "010afeffffff0f"],
  ["signed minimum", [-2147483648], "010affffffff0f"],
  ["unsigned minimum", [2147483648], "010300000080"],
  ["unsigned maximum", [4294967295], "0103ffffffff"],
  ["large integer", [4294967296], "010441f0000000000000"],
  ["fraction", [1.5], "01043ff8000000000000"],
  ["negative fraction", [-1.5], "0104bff8000000000000"],
  ["empty string", [""], "010200"],
  ["Latin-1", ["\xE9"], "010901e9"],
  ["UTF-16", ["\u0100"], "0101010001"],
  ["surrogate pair", ["\u{1F600}"], "0101023dd800de"],
  ["unpaired surrogate", ["\uD800"], "01010100d8"],
  ["base64", ["AQ=="], "01020101"],
  ["base64 pad bits", ["AB=="], "01090441423d3d"],
  ["base64 padding", ["A==="], "010904413d3d3d"],
  ["base64 whitespace", ["AQ== "], "01090541513d3d20"],
  ["nested arrays", [1, [true, false], null], "030a020702060500"]
];
for (const [name, values, expected] of cases) {
  test(`serializer preserves ${name}`, () => {
    assert.equal(serializeValues(values).toString("hex"), expected);
  });
}
function refuses(value) {
  assert.throws(() => serializeValues(value), (error) => error instanceof CastleError && error.code === "encoder" && error.message === "Castle Library encode-token failed");
}
test("serializer matches 2048 cases from the immutable Rust implementation", () => {
  const digest = createHash("sha256");
  const bits = Buffer.alloc(8);
  let state = 0x123456789abcdef0n;
  let size = 0;
  for (let index = 0; index < 2048; index += 1) {
    state = BigInt.asUintN(64, state * 6364136223846793005n + 1442695040888963407n);
    bits.writeBigUInt64BE(state);
    const text = String.fromCharCode(
      index,
      Number(state >> 32n & 65535n),
      Number(state >> 16n & 65535n),
      Number(state & 65535n)
    );
    const bytes = serializeValues([bits.readDoubleBE(), text, [index % 2 === 0, null, void 0, index - 1024]]);
    digest.update(bytes);
    size += bytes.length;
  }
  assert.equal(size, 57200);
  assert.equal(digest.digest("hex"), "df6e46f93d7070d490623e163e7f6d450d3345fc2ffd7b9d4976a17ef04b9a01");
});
test("serializer rejects unsupported values and active objects with fixed errors", () => {
  for (const value of [null, {}, "text", 1, [1n], [Symbol()], [() => {
  }], [{}]]) refuses(value);
  refuses(new Proxy([], { get() {
    throw new Error("synthetic-secret");
  } }));
  const getter = [];
  Object.defineProperty(getter, 0, { get() {
    throw new Error("synthetic-secret");
  } });
  refuses(getter);
  const cycle = [];
  cycle.push(cycle);
  refuses(cycle);
});
test("serializer bounds total value count before expanding arrays", () => {
  assert.equal(serializeValues(Array(4095).fill(null)).length, 4097);
  assert.ok(serializeValues([Array(4094).fill(null)]).length > 4094);
  refuses(Array(4096).fill(null));
  refuses([Array(4095).fill(null)]);
});
test("serializer bounds total UTF-16 units without dropping unpaired surrogates", () => {
  assert.equal(serializeValues(["\uD800".repeat(65536)]).length, 131077);
  assert.ok(serializeValues(["a".repeat(32768), "b".repeat(32768)]).length > 0);
  refuses(["a".repeat(65537)]);
  refuses(["a".repeat(32768), "b".repeat(32769)]);
});
test("serializer bounds nesting and preserves frozen inputs", () => {
  let value = null;
  for (let depth = 0; depth < 63; depth += 1) value = Object.freeze([value]);
  assert.ok(serializeValues(Object.freeze([value])).length > 0);
  refuses([[value]]);
  const input = Object.freeze([1, Object.freeze([true, false]), null]);
  assert.equal(serializeValues(input).toString("hex"), "030a020702060500");
  assert.deepEqual(input, [1, [true, false], null]);
});
