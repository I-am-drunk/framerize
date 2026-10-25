#!/usr/bin/env python3
"""Compare the native SharedWorker reducer with the pinned SDK's pure Qon."""
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
MARKERS = ["83au4jz", "3mxkmwkle0", "9q5p9p15pn", "99j32wg9x",
           "gp9vlcc", "2v315vt89", "vrbhklehy", "ydlkmari0n"]


def message(value, tag=None):
    return array([number(2) if tag is None else tag] + [UNDEFINED] * 9 + [value])


def cases():
    result = []
    texts = MARKERS + ["", "2", "plain version", "Ā", "\ud800", "😀", "\0"]
    texts += [text + "x" for text in MARKERS]
    for i, text in enumerate(texts):
        result.append((f"string_{i}", string(utf16(text))))
    other = [NULL, UNDEFINED, FALSE, TRUE, number(0), number(2), number(math.nan),
             number(math.inf), object_value([]),
             object_value([("0", number(2)), ("10", string(utf16("version")))])]
    for i, value in enumerate(other):
        result.append((f"non_array_{i}", value))
    tags = [NULL, UNDEFINED, FALSE, TRUE, number(0), number(-0.0), number(1),
            number(3), number(-2), number(math.nextafter(2, math.inf)),
            number(math.nan), number(math.inf), string(utf16("2")),
            array([number(2)]), object_value([])]
    for i, tag in enumerate(tags):
        result.append((f"tag_{i}", message(string(utf16("present")), tag)))
    result.append(("empty_message", array([])))
    for length in range(1, 11):
        result.append((f"absent_slot_{length}", array([number(2)] + [TRUE] * (length - 1))))
    fields = [NULL, UNDEFINED, FALSE, TRUE, number(0), number(-0.0),
              number(math.nan), number(math.inf), number(-math.inf),
              number(5e-324), number(-5e-324), number(2), number(-1),
              array([]), array([NULL, UNDEFINED, FALSE]), object_value([]),
              object_value([("nested", array([FALSE, string([0xD800])]))])]
    fields += [string(utf16(text)) for text in texts]
    for i, value in enumerate(fields):
        result.append((f"selected_slot_{i}", message(value)))
    result.append(("neighbor_slots", array([number(2)] + [UNDEFINED] * 8 +
                  [string(utf16("wrong9")), string(utf16("right10")),
                   string(utf16("wrong11"))])))
    return result


def check():
    selected = cases()
    reference = subprocess.run([os.environ.get("NODE", "node"),
        str(ROOT / "scripts/castle_v3_worker_result_reference.mjs")],
        input=json.dumps([value[0] for _, value in selected]), text=True,
        capture_output=True, check=True, timeout=10)
    expected = json.loads(reference.stdout)
    assert [pair[0] for pair in expected["pairs"]] == MARKERS
    assert len(expected["bytes"]) == len(selected)
    rows = []
    for (name, (_, expression)), data in zip(selected, expected["bytes"]):
        assert all(type(byte) is int and 0 <= byte <= 255 for byte in data)
        rows.append(f'({json.dumps(name)},{expression},&{data})')
    module = json.dumps(str(ROOT / "crates/framer-native/src/castle/mod.rs"))
    rows_text = ",\n".join(rows)
    source = f'''#[allow(dead_code)] #[path = {module}] mod castle;
use castle::v3::{{serialize, shared_worker_version, Value}};
#[test]
fn matches_pinned_sdk_worker_reducer() {{
    let cases: [(&str, Value, &[u8]); {len(rows)}] = [{rows_text}];
    for (name, input, expected) in cases {{
        let value = shared_worker_version(input).unwrap();
        assert_eq!(serialize(&[value]).unwrap(), expected, "{{name}}");
    }}
}}
#[test]
fn refuses_normalized_selected_values() {{
    for value in [Value::JsonFailure, Value::JsonFallback(vec![48]),
                  Value::JsonFallback(vec![110, 117, 108, 108])] {{
        let mut input = vec![Value::Number(2.0)];
        input.extend((1..10).map(|_| Value::Undefined));
        input.push(value);
        assert!(shared_worker_version(Value::Array(input)).is_err());
    }}
}}
'''
    with tempfile.TemporaryDirectory(prefix="castle-v3-worker-result-") as directory:
        path = Path(directory)
        (path / "check.rs").write_text(source)
        compile_fixture = runpy.run_path(str(ROOT / "scripts/v3_native_fixture.py"))["compile_fixture"]
        compile_fixture(path / "check.rs", path / "check")
        subprocess.run([str(path / "check"), "--test-threads=1"], check=True, timeout=10)
    print(f"PASS: {len(selected)} source-exact worker reduction/serialization comparisons")
    print("Supplied synthetic values only; no worker, SDK startup, accounts or network")


if __name__ == "__main__":
    check()
