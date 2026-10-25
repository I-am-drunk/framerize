#!/usr/bin/env python3
"""Compare native JSON hashing with the pinned SDK's original UP function."""
import json
import math
import os
from pathlib import Path
import re
import runpy
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
shared = runpy.run_path(str(ROOT / "scripts/check_v3_objects.py"))
number, string, array, utf16 = [shared[name] for name in ("number", "string", "array", "utf16")]


def cases():
    result = []
    for name, original, native in shared["cases"]():
        if not name.startswith("json_number_") or int(name.rsplit("_", 1)[1]) < 15:
            result.append((name, {"kind": "array", "values": original},
                           "Value::Array(vec![" + ",".join(native) + "])"))
    scalars = [shared["NULL"], shared["UNDEFINED"], number(0), number(-0.0),
               number(math.nan), number(math.inf), number(-math.inf), number(1e21)]
    scalars += [string(utf16(text)) for text in ["", "Ā", "😀", "\ud800", "\udc00", "\0"]]
    for i, (original, native) in enumerate(scalars):
        result.append((f"scalar_{i}", original, native))
    for kind in ("function", "symbol", "cyclic", "throwing", "bigint"):
        native = "Value::Undefined" if kind in ("function", "symbol") else "Value::JsonFailure"
        result.append((f"failure_{kind}", {"kind": kind, "value": "1234"}, native))
    for i in range(64):
        original, native = string(utf16(f"padding-{i}"))
        result.append((f"padding_{i}", original, native))
    return result


def check():
    selected = cases()
    reference = subprocess.run([os.environ.get("NODE", "node"),
        str(ROOT / "scripts/castle_v3_hash_reference.mjs")],
        input=json.dumps([value for _, value, _ in selected]), text=True,
        capture_output=True, check=True, timeout=10)
    expected = json.loads(reference.stdout)
    assert expected["aliases"] == ["imul", "toString", "padStart", "0"]
    assert len(expected["rows"]) == len(selected)
    rows = []
    for (name, _, expression), outcome in zip(selected, expected["rows"]):
        if "error" in outcome:
            assert outcome["error"] in ("TypeError", "Error"), (name, outcome)
            value = "None"
        else:
            assert re.fullmatch("[0-9a-f]{8}", outcome["hash"])
            value = f'Some({json.dumps(outcome["hash"])})'
        rows.append(f'({json.dumps(name)},{expression},{value})')
    assert any(row.get("hash", "").startswith("0") for row in expected["rows"])
    module = json.dumps(str(ROOT / "crates/framer-native/src/castle/mod.rs"))
    rows_text = ",\n".join(rows)
    source = f'''#![recursion_limit="1024"]
#[allow(dead_code)] #[path = {module}] mod castle;
use castle::v3::{{json_hash, Value}};
#[test]
fn matches_pinned_sdk_json_hash() {{
    let cases: [(&str, Value, Option<&str>); {len(rows)}] = [{rows_text}];
    for (name, input, expected) in cases {{
        assert_eq!(json_hash(&input).as_deref(), expected, "{{name}}");
        assert_eq!(json_hash(&input).as_deref(), expected, "repeat {{name}}");
    }}
}}
'''
    with tempfile.TemporaryDirectory(prefix="castle-v3-hash-") as directory:
        path = Path(directory)
        (path / "check.rs").write_text(source)
        compile_fixture = runpy.run_path(str(ROOT / "scripts/v3_native_fixture.py"))["compile_fixture"]
        compile_fixture(path / "check.rs", path / "check")
        subprocess.run([str(path / "check"), "--test-threads=1"], check=True, timeout=10)
    errors = sum("error" in row for row in expected["rows"])
    print(f"PASS: {len(selected)} JSON hash comparisons, including {errors} refused conversions")
    print("Synthetic values only; no SDK startup, network observations or accounts")


if __name__ == "__main__":
    check()
