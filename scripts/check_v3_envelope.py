#!/usr/bin/env python3
"""Compare native output wrapping with the pinned SDK, using synthetic inputs."""
import importlib.util
import json
import os
import runpy
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
TIMESTAMPS = (0, 1, -1, 9, 10, 99, 100, 999999999999, 1700000000123,
              2**31 - 1, 2**31, 2**32 - 1, 2**32, -2**32,
              8640000000000000, -8640000000000000)


def fixtures():
    spec = importlib.util.spec_from_file_location(
        "v3_compression_fixtures", ROOT / "scripts/check_v3_deflate.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    raw = [(name, data, TIMESTAMPS[i % len(TIMESTAMPS)])
           for i, (name, data) in enumerate(module.raw_cases())]
    for timestamp in TIMESTAMPS:
        for data in (b"", b"\x00", bytes(range(5)), bytes(range(65))):
            raw.append((f"clock_{timestamp}_{len(data)}", data, timestamp))
    pipeline = module.pipeline_cases()
    serialized = module.original("castle_v3_reference.mjs", [x[1] for x in pipeline])
    compressed = module.original("castle_v3_deflate_reference.mjs", serialized)
    return raw, pipeline, compressed


def check():
    raw, pipeline, compressed = fixtures()
    cases = [{"data": list(data), "timestamp": timestamp} for _, data, timestamp in raw]
    cases += [{"data": data, "timestamp": TIMESTAMPS[i % len(TIMESTAMPS)]}
              for i, data in enumerate(compressed)]
    result = subprocess.run(
        [os.environ.get("NODE", "node"), str(ROOT / "scripts/castle_v3_envelope_reference.mjs")],
        input=json.dumps({"cases": cases}), text=True, capture_output=True,
        check=True, timeout=20)
    expected = json.loads(result.stdout)
    assert len(expected) == len(cases)
    with tempfile.TemporaryDirectory(prefix="castle-v3-envelope-") as directory:
        path = Path(directory)
        raw_rows, pipeline_rows = [], []
        for index, (case, answer) in enumerate(zip(cases, expected)):
            output = path / f"{index}.expected"
            output.write_text(answer["value"], encoding="ascii")
            suffix = f'{case["timestamp"]},include_str!({json.dumps(str(output))}))'
            if index < len(raw):
                source = path / f"{index}.input"
                source.write_bytes(bytes(case["data"]))
                raw_rows.append(f'({json.dumps(raw[index][0])},'
                                f'include_bytes!({json.dumps(str(source))}),{suffix}')
            else:
                name, _, expressions = pipeline[index - len(raw)]
                pipeline_rows.append(f'({json.dumps(name)},vec![{",".join(expressions)}],{suffix}')
        run_native(path, raw_rows, pipeline_rows)
    print(f"PASS: {len(raw)} envelope comparisons and {len(pipeline)} complete encoding cases")


def run_native(path, raw_rows, pipeline_rows):
    module = json.dumps(str(ROOT / "crates/framer-native/src/castle/mod.rs"))
    raw_text, pipeline_text = ",\n".join(raw_rows), ",\n".join(pipeline_rows)
    source = f'''#![recursion_limit="1024"]
#[allow(dead_code)] #[path = {module}] mod castle;
use castle::v3::{{deflate, envelope, serialize, EnvelopeError, Value}};
#[test]
fn matches_original_envelope() {{
    let cases: [(&str, &[u8], i64, &str); {len(raw_rows)}] = [{raw_text}];
    for (name, input, timestamp, expected) in cases {{
        assert_eq!(envelope(input, timestamp).unwrap(), expected, "{{name}}");
        assert_eq!(envelope(input, timestamp).unwrap(), expected, "repeat {{name}}");
    }}
}}
#[test]
fn complete_encoding_matches_original() {{
    let cases: [(&str, Vec<Value>, i64, &str); {len(pipeline_rows)}] = [{pipeline_text}];
    for (name, input, timestamp, expected) in cases {{
        let bytes = deflate(&serialize(&input).unwrap());
        assert_eq!(envelope(&bytes, timestamp).unwrap(), expected, "{{name}}");
    }}
}}
'''
    source += '''
#[test]
fn rejects_timestamps_outside_javascript_date_range() {
    for timestamp in [i64::MIN, -8640000000000001, 8640000000000001, i64::MAX] {
        assert_eq!(envelope(&[], timestamp), Err(EnvelopeError::TimestampOutsideDateRange));
    }
}
'''
    (path / "check.rs").write_text(source)
    compiler = runpy.run_path(str(ROOT / "scripts/v3_native_fixture.py"))["compile_fixture"]
    compiler(path / "check.rs", path / "check")
    subprocess.run([str(path / "check")], check=True, timeout=20)


if __name__ == "__main__":
    check()
