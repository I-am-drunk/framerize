#!/usr/bin/env python3
"""Compare native network-record reduction with the pinned SDK's krn/ud."""
import json
import math
import os
from pathlib import Path
import runpy
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
shared = runpy.run_path(str(ROOT / "scripts/check_v3_objects.py"))
number, string, array, utf16, object_value = [shared[name] for name in
    ("number", "string", "array", "utf16", "object_value")]
NULL, UNDEFINED = shared["NULL"], shared["UNDEFINED"]
TRUE = ({"kind": "boolean", "value": True}, "Value::Bool(true)")
FALSE = ({"kind": "boolean", "value": False}, "Value::Bool(false)")


def record(value):
    return array([array([UNDEFINED] * 38 + [value])])


def cases():
    result = []
    values = [NULL, UNDEFINED, FALSE, TRUE, number(0), number(-0.0),
              number(1), number(-1), number(math.nan), number(math.inf),
              number(-math.inf), number(5e-324), string([]), string(utf16("-")),
              string(utf16("192.0.2.1")), string([0xD800]), array([]),
              array([FALSE, UNDEFINED]), object_value([]),
              object_value([("value", string([0xD800]))])]
    for i, value in enumerate(values):
        result.append((f"field_{i}", record(value)))
        if value[0]["kind"] not in ("array", "object"):
            result.append((f"non_object_{i}", value))
    for i, value in enumerate([array([]), object_value([]), array([NULL]),
                               array([UNDEFINED]), object_value([("0", NULL)])]):
        result.append((f"missing_record_{i}", value))
    for i, row in enumerate([array([]), object_value([]), FALSE, TRUE, number(0),
                            string([]), string([65] * 38), string([65] * 38 + [0xD800])]):
        result.append((f"row_shape_{i}", array([row])))
    result.append(("own_numeric_properties", object_value([
        ("0", object_value([("38", string(utf16("192.0.2.2")))]))])))
    result.append(("duplicate_property", object_value([
        ("0", NULL), ("0", object_value([("38", NULL), ("38", TRUE)]))])))
    result.append(("neighbor_fields", array([array([NULL] * 37 +
        [string(utf16("wrong37")), string(utf16("right38")),
         string(utf16("wrong39"))]), NULL])))
    for length in (1, 37, 38, 39, 40):
        result.append((f"record_length_{length}", array([array([TRUE] * length)])))
    return result


def check():
    selected = cases()
    reference = subprocess.run([os.environ.get("NODE", "node"),
        str(ROOT / "scripts/castle_v3_network_reference.mjs")],
        input=json.dumps([value[0] for _, value in selected]), text=True,
        capture_output=True, check=True, timeout=10)
    expected = json.loads(reference.stdout)
    assert expected["kinds"] == ["object", "undefined", "function"]
    assert expected["getter"] == "function"
    assert len(expected["rows"]) == len(selected)
    rows = []
    for (name, (_, expression)), outcome in zip(selected, expected["rows"]):
        if "error" in outcome:
            assert outcome == {"error": "TypeError"}, (name, outcome)
            value = "None"
        else:
            data = outcome["bytes"]
            assert all(type(byte) is int and 0 <= byte <= 255 for byte in data)
            value = f"Some(&{data})"
        rows.append(f'({json.dumps(name)},{expression},{value})')
    module = json.dumps(str(ROOT / "crates/framer-native/src/castle/mod.rs"))
    rows_text = ",\n".join(rows)
    source = f'''#[allow(dead_code)] #[path = {module}] mod castle;
use castle::v3::{{serialize, network_address, NetworkValueError, Value}};
#[test]
fn matches_pinned_sdk_network_reducer() {{
    let cases: [(&str, Value, Option<&[u8]>); {len(rows)}] = [{rows_text}];
    for (name, input, expected) in cases {{
        match (network_address(input), expected) {{
            (Ok(value), Some(bytes)) => assert_eq!(serialize(&[value]).unwrap(), bytes, "{{name}}"),
            (Err(NetworkValueError::MissingRecord), None) => {{}},
            (actual, _) => panic!("{{name}}: unexpected {{actual:?}}"),
        }}
    }}
}}
#[test]
fn refuses_normalized_inputs_at_each_selected_level() {{
    for kind in 0..3 {{ for position in 0..3 {{
        let value = match kind {{
            0 => Value::JsonFailure,
            1 => Value::JsonFallback(vec![48]),
            _ => Value::JsonFallback(vec![110, 117, 108, 108]),
        }};
        let input = match position {{
            0 => value,
            1 => Value::Array(vec![value]),
            _ => {{
                let mut row: Vec<_> = (0..38).map(|_| Value::Undefined).collect();
                row.push(value); Value::Array(vec![Value::Array(row)])
            }},
        }};
        assert!(matches!(network_address(input), Err(NetworkValueError::NormalizedValue)));
    }} }}
}}
'''
    with tempfile.TemporaryDirectory(prefix="castle-v3-network-result-") as directory:
        path = Path(directory)
        (path / "check.rs").write_text(source)
        compile_fixture = runpy.run_path(str(ROOT / "scripts/v3_native_fixture.py"))["compile_fixture"]
        compile_fixture(path / "check.rs", path / "check")
        subprocess.run([str(path / "check"), "--test-threads=1"], check=True, timeout=10)
    errors = sum("error" in row for row in expected["rows"])
    print(f"PASS: {len(selected)} source-exact network reduction cases, including {errors} errors")
    print("Supplied synthetic observations only; no RTC, network, SDK startup or accounts")


if __name__ == "__main__":
    check()
