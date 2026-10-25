#!/usr/bin/env python3
"""Compare the native post-collection entry point with the original SDK stages."""
import json
import os
from pathlib import Path
import runpy
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def check():
    shared = runpy.run_path(str(ROOT/'scripts/check_v3_deflate.py'))
    cases = shared['pipeline_cases']()
    for name, make in [
        ('indexed_snapshot', lambda i: ({'kind': 'number', 'value': str(i)}, f'Value::Number({i}.0)')),
        ('sparse_snapshot', lambda i: ({'kind': 'undefined'}, 'Value::Undefined') if i < 764
         else ({'kind': 'string', 'units': [0xD800, 0, 0xDFFF]}, 'Value::String(vec![0xD800,0,0xDFFF])')),
        ('boolean_snapshot', lambda i: ({'kind': 'boolean', 'value': i % 2 == 0}, f'Value::Bool({str(i % 2 == 0).lower()})')),
    ]:
        pairs = [make(i) for i in range(765)]
        cases.append((name, [x[0] for x in pairs], [x[1] for x in pairs]))
    serialized = shared['original']('castle_v3_reference.mjs', [x[1] for x in cases])
    compressed = shared['original']('castle_v3_deflate_reference.mjs', serialized)
    clocks = [0, -1, 1700000000123, 8640000000000000]
    inputs = [{'data': data, 'timestamp': clocks[i % len(clocks)]} for i,data in enumerate(compressed)]
    result = subprocess.run([os.environ.get('NODE','node'), str(ROOT/'scripts/castle_v3_envelope_reference.mjs')],
        input=json.dumps({'cases': inputs}), capture_output=True, text=True, check=True, timeout=20)
    expected = json.loads(result.stdout)
    assert len(expected) == len(cases)
    with tempfile.TemporaryDirectory(prefix='castle-v3-collected-') as directory:
        path = Path(directory)
        rows = []
        for i, ((name, _, values), answer) in enumerate(zip(cases, expected)):
            output = path/f'{i}.expected'
            output.write_text(answer['value'], encoding='ascii')
            rows.append(f'({json.dumps(name)},vec![{",".join(values)}],{inputs[i]["timestamp"]},'
                        f'include_str!({json.dumps(str(output))}))')
        module = json.dumps(str(ROOT/'crates/framer-native/src/castle/mod.rs'))
        source = f'''#![recursion_limit="2048"]
#[allow(dead_code,unused_imports)] #[path={module}] mod castle;
use castle::v3::{{encode_collected, EncodingError, EnvelopeError, Value}};
#[test] fn matches_original_post_collection_pipeline() {{
    let cases: [(&str,Vec<Value>,i64,&str);{len(rows)}]=[{','.join(rows)}];
    for (name,values,time,expected) in cases {{
        assert_eq!(encode_collected(&values,time).unwrap(),expected,"{{name}}");
    }}
}}
#[test] fn preserves_invalid_clock_refusal() {{
    for time in [i64::MIN,-8640000000000001,8640000000000001,i64::MAX] {{
        assert_eq!(encode_collected(&[],time),
            Err(EncodingError::Envelope(EnvelopeError::TimestampOutsideDateRange)));
    }}
}}
'''
        (path/'check.rs').write_text(source)
        compile_fixture=runpy.run_path(str(ROOT/'scripts/v3_native_fixture.py'))['compile_fixture']
        compile_fixture(path/'check.rs',path/'check')
        subprocess.run([str(path/'check'),'--test-threads=1'],check=True,timeout=20)
    print(f'PASS: {len(cases)} original-pipeline comparisons, including three 765-position synthetic arrays')


if __name__ == '__main__':
    check()
