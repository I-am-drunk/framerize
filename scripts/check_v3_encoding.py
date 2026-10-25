#!/usr/bin/env python3
"""Compare native encoding primitives with isolated, source-exact SDK helpers."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
SDK = ROOT / "reference/decompiled/castle/castleSDKV3-LNGA36I3.mjs"
SHA256 = "3337eba7ffe614c2ebd9b2b8a55794f2e2445a9b3658d1af5eb7530152b72c71"


def oracle_source(mode):
    data = SDK.read_bytes()
    if hashlib.sha256(data).hexdigest() != SHA256:
        raise ValueError("SDK differs from the reviewed source")
    aliases = [(314479, "t=0"), (314751, "ee=128"), (315603, "N=127"),
               (314549, "M=7"), (314497, 'I="length"'), (314530, "at=255")]
    aliases += [(314537, "o=1"), (315128, "nn=31"), (315852, "f=8"),
                (316022, "y=16"), (316116, "Pn=24")]
    extra = ""
    if mode == "utf16":
        aliases += [(314514, 'Qi="charCodeAt"'), (293463, "Zr=String")]
        aliases += [(292612, "Wt=10"), (292642, "Te=12"), (292587, "Cf=14"),
                    (292570, "s=16"), (292631, "Ln=18"), (292648, "jr=20"),
                    (292654, "S0=22"), (292660, "Ui=24"), (292666, "Ri=26"),
                    (292672, "kt=28"), (292678, "F3=30"), (292684, "K2=32"),
                    (292690, "wr=34"), (292696, "it=36")]
        extra = data[5783:8566].decode() + data[276160:276254].decode()
    for start, text in aliases:
        if data[start:start + len(text)] != text.encode():
            raise ValueError("SDK alias differs from the reviewed source")
    declarations = ",".join(text for _, text in aliases)
    writer = data[274997:275051].decode()
    sink = data[276131:276160].decode()
    calls = {
        "unsigned": "R(value)",
        "signed": "const W=value;" + data[275909:275928].decode(),
        "fixed": "(" + data[275842:275897].decode() + ")(value)",
        "utf16": "S(String.fromCharCode(...value))",
    }
    return (f"const {declarations}; inputs.map(value => {{const Cn=[];"
            f"{writer}{sink}{extra} {calls[mode]}; return Cn;}})")


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


def original_bytes(values, mode):
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
        input=json.dumps({"source": oracle_source(mode), "inputs": values}),
        text=True, capture_output=True, check=True, timeout=10)
    return json.loads(result.stdout)


def check_native(values, expected, mode):
    if len(expected) != len(values) or any(
        not isinstance(row, list)
        or not 1 <= len(row) <= (2 * len(value) + 5 if mode == "utf16" else 5)
        or any(type(byte) is not int or not 0 <= byte <= 255 for byte in row)
        for value, row in zip(values, expected)
    ):
        raise ValueError("Invalid source oracle output")
    module = json.dumps(str(ROOT / "crates/framer-native/src/castle/mod.rs"))
    rows = ",\n".join(f"({'&' if mode == 'utf16' else ''}{value}, &{row})"
                      for value, row in zip(values, expected))
    value_type, call = {
        "unsigned": ("u32", "write_unsigned(value, &mut output)"),
        "signed": ("i32", "write_signed(value, &mut output)"),
        "fixed": ("u32", "write_fixed_unsigned(value, &mut output)"),
        "utf16": ("&[u16]", "write_utf16(value, &mut output).unwrap()"),
    }[mode]
    source = f'''#[allow(dead_code)] #[path = {module}] mod castle;
#[test]
fn matches_pinned_sdk_{mode}_writer() {{
    let cases: &[({value_type}, &[u8])] = &[{rows}];
    for &(value, expected) in cases {{
        let mut output = vec![123, 45];
        castle::v3::{call};
        assert_eq!(&output[..2], &[123, 45]);
        assert_eq!(&output[2..], expected, "value={{value:?}}");
        let end = output.len();
        castle::v3::{call};
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
    signed = [v if v < 0x80000000 else v - 0x100000000 for v in selected]
    signed += [sign * ((1 << bit) + delta) for bit in (6, 13, 20, 27)
               for delta in (-1, 0, 1) for sign in (-1, 1)]
    strings = [[], [0], [65], [255], [256], [0xFFFF], [0xD83D, 0xDE00],
               [0xD800], [0xDC00], [0xDC00, 0xD800], [65, 0, 0xD800, 66]]
    strings += [list(range(length)) for length in (126, 127, 128, 129, 255, 256)]
    strings += [[(n * 7919 + offset) & 0xFFFF for n in range(257)]
                for offset in (0, 127, 255, 0xD800)]
    for mode, values in (("unsigned", selected), ("signed", signed),
                         ("fixed", selected), ("utf16", strings)):
        check_native(values, original_bytes(values, mode), mode)
        print(f"PASS: {mode}: {len(values)} source comparisons", flush=True)
    print(f"SDK SHA256: {SHA256}; isolated original helper functions only")
    print("No full SDK, tokens, network, accounts or memory claim")
