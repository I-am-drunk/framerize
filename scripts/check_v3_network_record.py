#!/usr/bin/env python3
"""Compare record assembly with the SDK, using its observed collation order."""
import json
import math
import os
from pathlib import Path
import runpy
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
shared = runpy.run_path(str(ROOT / "scripts/check_v3_objects.py"))
number, string, utf16 = [shared[name] for name in ("number", "string", "utf16")]
NAMES = ["kind", "protocol", "address_class", "related_address_class",
         "port_class", "tcp_type", "component"]


def candidate(*fields):
    return [string(utf16(value)) if isinstance(value, str) else value for value in fields]


def cases():
    result = []

    def add(name, candidates=(), histories=((), (), ())):
        result.append((name, list(candidates), histories))

    add("empty")
    for field in range(7):
        values = [""] * 7
        values[field] = f"field{field}"
        add(f"field_path_{field}", [candidate(*values)])
    a = candidate("host", "udp", "private", "", "high", "", "1")
    b = candidate("srflx", "tcp", "public", "private", "low", "passive", "2")
    add("mixed", [b, a], (["new", "connected"], ["gathering"], ["stable"]))
    add("duplicate", [a, b, a])
    for count in (0, 1, 15, 16, 17, 33):
        rows = [candidate(f"type{count-i}", "udp", "", "", str(i), "", "") for i in range(count)]
        histories = tuple([f"state-{i}" for i in range(count)] for _ in range(3))
        add(f"bounds_{count}", rows, histories)
    protocols = ["", "__proto__", "constructor", "toString", "2", "10", "Ā", "\ud800"]
    add("protocol_keys", [candidate("", protocol, "", "", "", "", "") for protocol in protocols])
    add("collation", [candidate(text, "udp", "", "", "", "", "")
                      for text in ["a", "A", "á", "ä", "2", "10", "-", "_"]])
    add("literal_separators", [candidate("a:b", "c/d", "e|f", "+", "", "", "")])
    add("history_order", [a], (["z", "a", "z"], ["B", "a"], ["\ud800", "😀"]))
    scalars = [shared["NULL"], shared["UNDEFINED"], number(0), number(-0.0),
               number(math.nan), number(math.inf), number(-math.inf), number(1.25),
               ({"kind": "boolean", "value": False}, "Value::Bool(false)"),
               ({"kind": "boolean", "value": True}, "Value::Bool(true)")]
    for i, value in enumerate(scalars):
        add(f"scalar_{i}", [candidate(*([value] * 7))])
    add("unread_after_cap", [a] * 16 + [candidate(shared["object_value"]([]), *( [""] * 6))])
    return result


def check():
    selected = cases()
    inputs = [{"address": "192.0.2.1", "related": "", "candidates": [[v[0] for v in row] for row in rows],
               "histories": histories} for _, rows, histories in selected]
    reference = subprocess.run([os.environ.get("NODE", "node"), str(ROOT / "scripts/castle_v3_record_reference.mjs")],
        input=json.dumps(inputs), text=True, capture_output=True, check=True, timeout=10)
    expected = json.loads(reference.stdout)
    assert expected["fields"] == [38, 40, 18, 15, 24, 54, 30, 55, 47]
    assert len(expected["rows"]) == len(selected)

    def texts(values):
        return "vec![" + ",".join(f"vec!{utf16(value)}" for value in values) + "]"

    rows = []
    for (name, candidates, histories), outcome in zip(selected, expected["rows"]):
        candidate_code = ",".join("NetworkCandidate{" + ",".join(
            f"{key}:{value[1]}" for key, value in zip(NAMES, row)) + "}" for row in candidates)
        observation = "NetworkObservation{" + f"address:vec!{utf16('192.0.2.1')},related_address:vec![]," + \
            f"candidates:vec![{candidate_code}],connection_states:{texts(histories[0])}," + \
            f"gathering_states:{texts(histories[1])},signaling_states:{texts(histories[2])}" + "}"
        order = "vec![" + ",".join(f"vec!{units}" for units in outcome["order"]) + "]"
        data = outcome["bytes"]
        assert all(type(byte) is int and 0 <= byte <= 255 for byte in data)
        rows.append(f'({json.dumps(name)},{observation},{order},&{data})')
    module = json.dumps(str(ROOT / "crates/framer-native/src/castle/mod.rs"))
    rows_text = ",\n".join(rows)
    source = f'''#![recursion_limit="1024"]
#[allow(dead_code)] #[path = {module}] mod castle;
use castle::v3::{{assemble_network_record, NetworkCandidate, NetworkObservation,
    NonScalarCandidate, serialize, Value}};
#[test]
fn matches_pinned_sdk_record_with_supplied_collation() {{
    let cases: [(&str, NetworkObservation, Vec<Vec<u16>>, &[u8]); {len(rows)}] = [{rows_text}];
    for (name, observation, order, expected) in cases {{
        let value = assemble_network_record(observation, |left, right| {{
            let a = order.iter().position(|value| value == left).expect("selected signature");
            let b = order.iter().position(|value| value == right).expect("selected signature");
            a.cmp(&b)
        }}).unwrap();
        assert_eq!(serialize(&[value]).unwrap(), expected, "{{name}}");
    }}
}}
#[test]
fn refuses_non_scalar_candidate() {{
    let candidate = NetworkCandidate {{ kind:Value::Array(vec![]),
        protocol:Value::Undefined, address_class:Value::Undefined,
        related_address_class:Value::Undefined, port_class:Value::Undefined,
        tcp_type:Value::Undefined, component:Value::Undefined }};
    let observation = NetworkObservation {{ address:vec![], related_address:vec![],
        candidates:vec![candidate], connection_states:vec![],
        gathering_states:vec![], signaling_states:vec![] }};
    assert!(matches!(assemble_network_record(observation, |a,b| a.cmp(b)), Err(NonScalarCandidate)));
}}
'''
    with tempfile.TemporaryDirectory(prefix="castle-v3-record-") as directory:
        path = Path(directory)
        (path / "check.rs").write_text(source)
        compile_fixture = runpy.run_path(str(ROOT / "scripts/v3_native_fixture.py"))["compile_fixture"]
        compile_fixture(path / "check.rs", path / "check")
        subprocess.run([str(path / "check"), "--test-threads=1"], check=True, timeout=10)
    print(f"PASS: {len(selected)} record comparisons with source-runtime collation supplied")
    print("Collation, event collection and live acceptance are not implemented by this check")


if __name__ == "__main__":
    check()
