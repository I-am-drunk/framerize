#!/usr/bin/env python3
"""Compare native DEFLATE with the pinned fallback, including serialization."""
import importlib.util
import json
import os
import runpy
from pathlib import Path
import subprocess
import tempfile
import zlib

ROOT = Path(__file__).resolve().parents[1]


def raw_cases():
    cases = []
    random = bytearray()
    state = 1
    for _ in range(32768):
        state = (state * 1664525 + 1013904223) & 0xFFFFFFFF
        random.append(state >> 24)
    for length in (0, 1, 2, 3, 4, 15, 31, 127, 128, 257, 258, 259, 1023, 1024, 1025, 2048, 4096):
        cases += [(f"zero_{length}", bytes(length)),
                  (f"cycle_{length}", bytes(i % 256 for i in range(length))),
                  (f"random_{length}", bytes(random[:length])),
                  (f"phrase_{length}", (b"source exact compression " * 200)[:length])]
    for distance in (1023, 1024, 1025):
        prefix = bytes(random[:distance])
        cases.append((f"window_{distance}", prefix + prefix[:258]))
    cases += [("hash_chain", b"".join(b"aaa" + bytes([i]) for i in range(80)) * 4),
              ("long_random", bytes(random)), ("long_repeat", b"abc" * 22000),
              ("skewed", b"".join(bytes([i]) * (i * i + 1) for i in range(64)))]
    unique = {}
    for name, data in cases:
        unique.setdefault(data, name)
    return [(name, data) for data, name in unique.items()]


def original(script, inputs):
    result = subprocess.run([os.environ.get("NODE", "node"), str(ROOT / "scripts" / script)],
                            input=json.dumps(inputs), text=True, capture_output=True,
                            check=True, timeout=20)
    return json.loads(result.stdout)["bytes"]


def pipeline_cases():
    path = ROOT / "scripts/check_v3_serialization.py"
    spec = importlib.util.spec_from_file_location("v3_fixture_cases", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    names = {"empty", "null_and_booleans", "number_0", "number_3", "number_5",
             "binary_0", "binary_128", "latin1_129", "wide_128", "array_128",
             "outer_128", "nested_2", "nested_16", "supplied_json_fallback"}
    selected = [case for case in module.cases() if case[0] in names]
    assert len(selected) == len(names)
    return selected


def check():
    raw = raw_cases()
    pipeline = pipeline_cases()
    serialized = original("castle_v3_reference.mjs", [case[1] for case in pipeline])
    inputs = [data for _, data in raw] + [bytes(data) for data in serialized]
    expected = original("castle_v3_deflate_reference.mjs", [list(data) for data in inputs])
    assert len(inputs) == len(expected)
    for data, compressed in zip(inputs, expected):
        assert zlib.decompress(bytes(compressed), -15) == data
    with tempfile.TemporaryDirectory(prefix="castle-v3-deflate-") as directory:
        path = Path(directory)
        include = lambda p: f"include_bytes!({json.dumps(str(p))})"
        raw_rows, pipeline_rows = [], []
        for index, (data, compressed) in enumerate(zip(inputs, expected)):
            source, result = path / f"{index}.input", path / f"{index}.expected"
            source.write_bytes(data)
            result.write_bytes(bytes(compressed))
            if index < len(raw):
                raw_rows.append(f"({json.dumps(raw[index][0])},{include(source)},{include(result)})")
            else:
                name, _, expressions = pipeline[index - len(raw)]
                pipeline_rows.append(f"({json.dumps(name)},vec![{','.join(expressions)}],{include(result)})")
        run_native(path, raw_rows, pipeline_rows)
    print(f"PASS: {len(raw)} raw inputs and {len(pipeline)} serialization/compression cases")
    print("All reference streams also independently decompressed to the original bytes")


def run_native(path, raw_rows, pipeline_rows):
    module = json.dumps(str(ROOT / "crates/framer-native/src/castle/mod.rs"))
    raw_text, pipeline_text = ",\n".join(raw_rows), ",\n".join(pipeline_rows)
    source = f'''#![recursion_limit="1024"]
#[allow(dead_code)] #[path = {module}] mod castle;
use castle::v3::{{deflate, serialize, Value}};
#[test]
fn matches_original_fallback() {{
    let cases: [(&str, &[u8], &[u8]); {len(raw_rows)}] = [{raw_text}];
    for (name, input, expected) in cases {{
        assert_eq!(deflate(input), expected, "{{name}}");
        assert_eq!(deflate(input), expected, "repeat {{name}}");
    }}
}}
#[test]
fn serialization_and_compression_match_original() {{
    let cases: [(&str, Vec<Value>, &[u8]); {len(pipeline_rows)}] = [{pipeline_text}];
    for (name, input, expected) in cases {{
        assert_eq!(deflate(&serialize(&input).unwrap()), expected, "{{name}}");
    }}
}}
'''
    (path / "check.rs").write_text(source)
    compiler = runpy.run_path(str(ROOT / "scripts/v3_native_fixture.py"))["compile_fixture"]
    compiler(path / "check.rs", path / "check")
    subprocess.run([str(path / "check")], check=True, timeout=20)


if __name__ == "__main__":
    check()
