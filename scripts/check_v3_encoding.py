#!/usr/bin/env python3
"""Compare native u32 encoding with exact, isolated SDK leaf functions."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SDK = ROOT / "reference/decompiled/castle/castleSDKV3-LNGA36I3.mjs"
SHA256 = "3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71"


def oracle_source():
    data = SDK.read_bytes()
    if hashlib.sha256(data).hexdigest() != SHA256:
        raise ValueError("SDK differs from the reviewed source")
    aliases = [(314479, "t=0"), (314751, "ee=128"), (315603, "N=127"),
               (314549, "M=7"), (314497, 'I="length"'), (314530, "at=255")]
    for start, text in aliases:
        if data[start:start + len(text)] != text.encode():
            raise ValueError("SDK alias differs from the reviewed source")
    declarations = ",".join(text for _, text in aliases)
    writer = data[274997:275051].decode()
    sink = data[276131:276160].decode()
    return (f"const {declarations}; inputs.map(value => {{const Cn=[];"
            f"{writer}{sink} R(value); return Cn;}})")


def inputs():
    values = {0, 1, 0xFFFFFFFF}
    for bit in range(1, 33):
        for delta in (-1, 0, 1):
            value = (1 << bit) + delta
            if value <= 0xFFFFFFFF:
                values.add(value)
    state = 0
    for _ in range(4096):
        state = (1664525 * state + 1013904223) & 0xFFFFFFFF
        values.add(state)
    return sorted(values) + [128, 0, 0xFFFFFFFF, 1, 128]


def original_bytes(values):
    program = """
import vm from 'node:vm';
import {readFileSync} from 'node:fs';
const {source, inputs} = JSON.parse(readFileSync(0, 'utf8'));
const result = vm.runInNewContext(source, {inputs}, {
  timeout: 1000, contextCodeGeneration: {strings: false, wasm: false}
});
process.stdout.write(JSON.stringify(result));
"""
    result = subprocess.run(
        [os.environ.get("NODE", "node"), "--input-type=module", "-e", program],
        input=json.dumps({"source": oracle_source(), "inputs": values}),
        text=True, capture_output=True, check=True, timeout=10)
    return json.loads(result.stdout)


def check_native(values, expected):
    if len(expected) != len(values) or any(
        not isinstance(row, list) or not 1 <= len(row) <= 5
        or any(type(byte) is not int or not 0 <= byte <= 255 for byte in row)
        for row in expected
    ):
        raise ValueError("Invalid source oracle output")
    module = json.dumps(str(ROOT / "crates/framer-native/src/castle/mod.rs"))
    rows = ",\n".join(f"({value}, &{row})" for value, row in zip(values, expected))
    source = f'''#[path = {module}] mod castle;
#[test]
fn matches_pinned_sdk_unsigned_writer() {{
    let cases: &[(u32, &[u8])] = &[{rows}];
    for &(value, expected) in cases {{
        let mut output = vec![123, 45];
        castle::v3::write_unsigned(value, &mut output);
        assert_eq!(&output[..2], &[123, 45]);
        assert_eq!(&output[2..], expected, "value={{value}}");
        let end = output.len();
        castle::v3::write_unsigned(value, &mut output);
        assert_eq!(&output[2..end], expected);
        assert_eq!(&output[end..], expected);
    }}
}}
'''
    with tempfile.TemporaryDirectory(prefix="castle-v3-encoding-") as directory:
        path = Path(directory)
        (path / "check.rs").write_text(source)
        subprocess.run([os.environ.get("RUSTC", "rustc"), "--edition=2021",
                        "--test", str(path / "check.rs"), "-o", str(path / "check")],
                       check=True, timeout=30)
        subprocess.run([str(path / "check")], check=True, timeout=10)


if __name__ == "__main__":
    selected = inputs()
    check_native(selected, original_bytes(selected))
    print(f"PASS: {len(selected)} source comparisons; fresh and appended output")
    print(f"SDK SHA256: {SHA256}; only R/G and six literal aliases evaluated")
    print("Scope: u32 only; no full SDK, tokens, network, accounts or memory claim")
