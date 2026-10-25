#!/usr/bin/env python3
"""Compare native object JSON normalization with the pinned SDK serializer."""
import math
from pathlib import Path
import runpy
import struct

ROOT = Path(__file__).resolve().parents[1]
shared = runpy.run_path(str(ROOT / "scripts/check_v3_serialization.py"))
number, string, array, utf16 = [shared[name] for name in ("number", "string", "array", "utf16")]
NULL = ({"kind": "null"}, "Value::Null")
UNDEFINED = ({"kind": "undefined"}, "Value::Undefined")


def object_value(entries):
    original = {"kind": "object", "entries": [[key, value[0]] for key, value in entries]}
    native = "Value::Object(vec![" + ",".join(
        f"(vec!{utf16(key)},{value[1]})" for key, value in entries) + "])"
    return original, native


def cases():
    result = []

    def add(name, *values):
        result.append((name, [v[0] for v in values], [v[1] for v in values]))

    add("empty_object", object_value([]))
    add("scalars", object_value([("null", NULL), ("bool", ({"kind": "boolean", "value": True},
        "Value::Bool(true)")), ("array", array([UNDEFINED, NULL, number(-0.0)]))]))
    keys = ["b", "10", "2", "01", "0", "4294967295", "4294967294", "-0", "a",
            "1", "1.0", "", "-1", "1e0", "00", "__proto__", "constructor", " 1"]
    add("property_order", object_value([(key, number(i)) for i, key in enumerate(keys)]))
    add("duplicates", object_value([("a", number(1)), ("b", number(2)),
        ("a", number(3)), ("b", UNDEFINED), ("1", number(4)), ("1", number(5))]))
    for kind in ("undefined", "function", "symbol"):
        value = ({"kind": kind}, "Value::Undefined")
        add(f"omission_{kind}", value, object_value([("before", NULL), ("omit", value),
            ("array", array([value])), ("after", number(1))]))
    for kind in ("throwing", "cyclic", "bigint"):
        value = ({"kind": kind, "value": "9007199254740993"}, "Value::JsonFailure")
        add(f"failure_{kind}", value, object_value([("before", number(1)),
            ("nested", array([object_value([("fail", value)])]))]), number(2))
    texts = [chr(i) for i in range(32)] + ['"', "\\", "/", "é", "Ā", "\u2028\u2029",
        "\ud800", "\udc00", "\ud800\ud800", "\udc00\ud800", "😀", "a\0\ud800b", ""]
    for i, value in enumerate(texts):
        add(f"escape_{i}", object_value([(value, string(utf16(value)))]))
    for count in (127, 128, 255, 256):
        add(f"length_{count}", object_value([("text", string([0xD800] * count)),
            ("array", array([NULL] * count))]))
    values = [math.nan, math.inf, -math.inf, -0.0, 0.0, 5e-324, -5e-324,
              1.7976931348623157e308, 1000000000000000128.0]
    for exponent in (-308, -100, -7, -6, -5, -1, 0, 1, 15, 16, 20, 21, 22, 100, 308):
        value = 10.0 ** exponent
        for neighbor in (math.nextafter(value, 0.0), value, math.nextafter(value, math.inf)):
            values.extend([neighbor, -neighbor])
    state = 1
    for _ in range(768):
        state = (state * 6364136223846793005 + 1) & ((1 << 64) - 1)
        values.append(struct.unpack(">d", state.to_bytes(8, "big"))[0])
    for i, value in enumerate(values):
        add(f"json_number_{i}", object_value([("number", number(value))]))
    nested = object_value([("leaf", string(utf16("value")))])
    for depth in range(65):
        if depth in (0, 1, 2, 16, 64):
            add(f"nested_{depth}", number(1), nested, NULL)
        nested = object_value([("level", number(depth)), ("next", array([nested]))])
    converted = {"kind": "tojson", "result": {"kind": "string", "units": utf16("converted")}}
    supplied = (converted, f"Value::JsonFallback(vec!{utf16(chr(34) + 'converted' + chr(34))})")
    add("supplied_conversion", supplied, object_value([("custom", supplied)]))
    return result


if __name__ == "__main__":
    shared["check"](cases())
