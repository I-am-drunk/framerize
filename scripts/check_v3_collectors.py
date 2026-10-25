#!/usr/bin/env python3
"""Check supplied collector results at the actual SDK Q assignment boundary."""
import json
import os
from pathlib import Path
import runpy
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def check():
    w = runpy.run_path(str(ROOT/'scripts/check_v3_worker_result.py'))
    n = runpy.run_path(str(ROOT/'scripts/check_v3_network_result.py'))
    text = lambda s: w['string'](w['utf16'](s))
    cases = [('worker', w['message'](text('actual version'))),
             ('worker', w['message'](w['UNDEFINED'])),
             ('worker', text('83au4jz')), ('worker', w['NULL']),
             ('network', n['record'](text('192.0.2.1'))),
             ('network', n['record'](text(''))),
             ('network', n['UNDEFINED']), ('network', n['array']([]))]
    result = subprocess.run([os.environ.get('NODE','node'),
        str(ROOT/'scripts/castle_v3_collector_reference.mjs')],
        input=json.dumps([dict(kind=kind,value=value[0]) for kind,value in cases]),
        capture_output=True, text=True, check=True, timeout=10)
    expected = json.loads(result.stdout)
    assert len(expected) == len(cases)
    rows = []
    for (kind, (_, expression)), outcome in zip(cases, expected):
        assert outcome.get('error') in (None, 'TypeError')
        variant = 'SharedWorker' if kind == 'worker' else 'Network'
        ok = str('error' not in outcome).lower()
        rows.append(f'(CollectorResult::{variant}({expression}),{ok},&{outcome["bytes"]})')
    module = json.dumps(str(ROOT/'crates/framer-native/src/castle/mod.rs'))
    source = f'''#[allow(dead_code,unused_imports)] #[path={module}] mod castle;
use castle::v3::{{Value,CollectorResult,apply_collector_result,serialize}};
fn base() -> Vec<Value> {{(1000..1765).map(|n|Value::Number(n as f64)).collect()}}
#[test] fn matches_original_assignments_and_preserves_other_positions() {{
    let cases:[(CollectorResult,bool,&[u8]);{len(rows)}]=[{','.join(rows)}];
    for (input,ok,expected) in cases {{
        let mut values=base();
        assert_eq!(apply_collector_result(&mut values,input).is_ok(),ok);
        assert_eq!(serialize(&values).unwrap(),expected);
    }}
}}
#[test] fn normalized_input_refusal_leaves_every_position_unchanged() {{
    let mut values=base(); let before=serialize(&values).unwrap();
    let mut worker:Vec<_>=(0..11).map(|_|Value::Undefined).collect();
    worker[0]=Value::Number(2.0); worker[10]=Value::JsonFailure;
    for input in [CollectorResult::SharedWorker(Value::Array(worker)),
                  CollectorResult::Network(Value::JsonFailure)] {{
        assert!(apply_collector_result(&mut values,input).is_err());
        assert_eq!(serialize(&values).unwrap(),before);
    }}
}}
#[test] fn incomplete_arrays_are_not_extended() {{
    for (length,input) in [(242,CollectorResult::SharedWorker(Value::Null)),
                          (473,CollectorResult::Network(Value::Null))] {{
        let mut values=base(); values.truncate(length);
        let before=serialize(&values).unwrap();
        assert!(apply_collector_result(&mut values,input).is_err());
        assert_eq!(values.len(),length); assert_eq!(serialize(&values).unwrap(),before);
    }}
}}
'''
    with tempfile.TemporaryDirectory(prefix='v3-collector-') as directory:
        path = Path(directory)
        (path/'check.rs').write_text(source)
        compile_fixture = runpy.run_path(str(ROOT/'scripts/v3_native_fixture.py'))['compile_fixture']
        compile_fixture(path/'check.rs',path/'check')
        subprocess.run([str(path/'check'),'--test-threads=1'],check=True,timeout=10)
    print(f'PASS: {len(cases)} full-array assignment comparisons; refusal preserves input')


if __name__ == '__main__':
    check()
