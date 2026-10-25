#!/usr/bin/env python3
"""Compare typed Rust serialization with the pinned original SDK serializer."""
import base64
import json
import math
import os
from pathlib import Path
import struct
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def number(value):
    literal = ("NaN" if math.isnan(value) else
               ("Infinity" if value > 0 else "-Infinity") if math.isinf(value)
               else repr(value))
    bits = struct.unpack(">Q", struct.pack(">d", value))[0]
    return {"kind": "number", "value": literal}, f"Value::Number(f64::from_bits({bits}))"


def string(units):
    return {"kind": "string", "units": units}, f"Value::String(vec!{units})"


def array(values):
    return ({"kind": "array", "values": [v[0] for v in values]},
            "Value::Array(vec![" + ",".join(v[1] for v in values) + "])")


def utf16(text):
    raw = text.encode("utf-16-le", errors="surrogatepass")
    return list(struct.unpack("<" + "H" * (len(raw) // 2), raw))


def cases():
    result = []

    def add(name, values):
        result.append((name, [v[0] for v in values], [v[1] for v in values]))

    nil = ({"kind": "null"}, "Value::Null")
    yes = ({"kind": "boolean", "value": True}, "Value::Bool(true)")
    no = ({"kind": "boolean", "value": False}, "Value::Bool(false)")
    add("empty", [])
    add("null_and_booleans", [nil, yes, no, ({"kind": "undefined"}, "Value::Null")])
    floats = [float("nan"), float("inf"), -float("inf"), -0.0, 0.0, 0.5, -0.5,
              5e-324, -5e-324, 1.7976931348623157e308]
    for power in (7, 15, 21, 28, 31, 32, 52, 53):
        for delta in (-1, 0, 1):
            floats.extend([float((1 << power) + delta), -float((1 << power) + delta)])
    state = 1
    for _ in range(256):
        state = (state * 6364136223846793005 + 1) & ((1 << 64) - 1)
        floats.append(struct.unpack(">d", state.to_bytes(8, "big"))[0])
    for index, value in enumerate(floats):
        add(f"number_{index}", [number(value)])
    texts = ["", "test", "abc", "AB==", "AAB=", "====", "AA=A", "AAAA=",
             "AA==AAAA", "AA-_", "AA==\n", " A==", "é", "Ā", "\0", "\ud800",
             "\udc00", "\udc00\ud800", "😀", "a\0\ud800b"]
    alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
    texts += ["A" + char + "==" for char in alphabet]
    texts += ["AA" + char + "=" for char in alphabet]
    for index, text in enumerate(texts):
        add(f"string_{index}", [string(utf16(text))])
    for count in (0, 1, 2, 3, 4, 126, 127, 128, 129, 255, 256):
        payload = bytes((n * 137 + 31) % 256 for n in range(count))
        add(f"binary_{count}", [string(utf16(base64.b64encode(payload).decode()))])
        add(f"latin1_{count}", [string([255] * count)])
        add(f"wide_{count}", [string([0xD800] * count)])
        add(f"array_{count}", [array([number(1.5)] * count), no])
        add(f"outer_{count}", [nil] * count)
    nested = string(utf16("leaf"))
    for depth in range(129):
        if depth in (0, 1, 2, 16, 128):
            add(f"nested_{depth}", [yes, nested, number(-1), no])
        nested = array([nil, nested, number(depth)])
    text = '{"text":"\\ud800","number":1,"bool":true,"null":null}'
    obj = {"kind": "object", "entries": [
        ["text", string([0xD800])[0]], ["number", number(1.0)[0]],
        ["omit", {"kind": "undefined"}], ["bool", yes[0]], ["null", nil[0]]
    ]}
    add("supplied_json_fallback", [(obj, f"Value::JsonFallback(vec!{utf16(text)})")])
    for kind in ("throwing", "function", "symbol"):
        add(f"normalized_{kind}", [({"kind": kind}, "Value::Null")])
    return result


def check(selected):
    reference = subprocess.run(
        [os.environ.get("NODE", "node"), str(ROOT / "scripts/castle_v3_reference.mjs")],
        input=json.dumps([item[1] for item in selected]), text=True,
        capture_output=True, check=True, timeout=10)
    expected = json.loads(reference.stdout)
    assert expected["types"] == ["string", "number", "boolean"]
    assert len(expected["bytes"]) == len(selected)
    rows = []
    for (name, _, expressions), data in zip(selected, expected["bytes"]):
        assert all(type(byte) is int and 0 <= byte <= 255 for byte in data)
        rows.append(f'({json.dumps(name)},vec![{",".join(expressions)}],&{data})')
    module = json.dumps(str(ROOT / "crates/framer-native/src/castle/mod.rs"))
    rows_text = ",\n".join(rows)
    source = f'''#![recursion_limit="1024"]
#[allow(dead_code)] #[path = {module}] mod castle;
use castle::v3::{{serialize, Value}};
#[test]
fn matches_pinned_sdk_serializer() {{
    let cases: [(&str, Vec<Value>, &[u8]); {len(rows)}] = [{rows_text}];
    for (name, input, expected) in cases {{
        assert_eq!(serialize(&input).unwrap(), expected, "{{name}}");
        assert_eq!(serialize(&input).unwrap(), expected, "repeat {{name}}");
    }}
}}
'''
    with tempfile.TemporaryDirectory(prefix="castle-v3-serializer-") as directory:
        path = Path(directory)
        (path / "check.rs").write_text(source)
        subprocess.run([os.environ.get("RUSTC", "rustc"), "--edition=2021",
                        "--test", str(path / "check.rs"), "-o", str(path / "check")],
                       check=True, timeout=30)
        subprocess.run([str(path / "check")], check=True, timeout=10)
    print(f"PASS: {len(selected)} complete array comparisons with the original serializer")
    print("Object fallback inputs use supplied JSON text or normalized null")
    print("No full SDK startup, tokens, network, accounts or memory claim")


if __name__ == "__main__":
    check(cases())
