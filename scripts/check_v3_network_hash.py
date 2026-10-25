#!/usr/bin/env python3
"""Compare the completed network-hash assignment with pinned Brn/Cr source."""
import json
import math
import os
from pathlib import Path
import random
import runpy
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]
shared = runpy.run_path(str(ROOT/'scripts/check_v3_network_result.py'))
array, string, number, utf16, object_value = [shared[n] for n in
    ('array', 'string', 'number', 'utf16', 'object_value')]
NULL, UNDEFINED, FALSE, TRUE = [shared[n] for n in
    ('NULL', 'UNDEFINED', 'FALSE', 'TRUE')]


def record(value):
    return array([array([UNDEFINED]*47+[value])])


def cases():
    texts = ['', '00000000', 'ffffffff', '01234567', 'a1b2c3d4',
             'r0yv7z6cx', '7e93ex445j', 'zye88asu', 'a9opjk', '2nrh3qm']
    units = [utf16(s) for s in texts]
    units += [[n] for n in (0, 1, 127, 128, 255, 256, 32767, 32768,
                            0xD7FF, 0xD800, 0xDBFF, 0xDC00, 0xDFFF, 0xFFFF)]
    units += [[0xD800, 0xDC00], [0xDC00, 0xD800], [0, 0xFFFF, 0xD800]]
    randomizer = random.Random(469)
    units += [[randomizer.randrange(65536) for _ in range(n)] for n in range(1, 49)]
    selected = [(f'text_{i}', record(string(v))) for i, v in enumerate(units)]
    for i, value in enumerate([NULL, UNDEFINED, FALSE, number(0), number(-0.0),
                                number(math.nan), string([])]):
        selected += [(f'missing_hash_{i}', record(value)), (f'outer_scalar_{i}', value)]
    for i, value in enumerate([array([]), array([NULL]), array([UNDEFINED]),
                                object_value([]), object_value([('0', NULL)])]):
        selected.append((f'missing_row_{i}', value))
    for i, row in enumerate([array([]), object_value([]), TRUE, number(1),
                            string([65]*48), string([0xD800]*48)]):
        selected.append((f'row_{i}', array([row])))
    selected.append(('own_property', object_value([('0', object_value([
        ('47', string(utf16('0123abcd')))]))])))
    return selected


def check():
    selected = cases()
    result = subprocess.run([os.environ.get('NODE', 'node'),
        str(ROOT/'scripts/castle_v3_collector_reference.mjs')],
        input=json.dumps([dict(kind='hash', compact=True, value=v[0]) for _, v in selected]),
        capture_output=True, text=True, check=True, timeout=10)
    expected = json.loads(result.stdout)
    assert len(selected) == len(expected)
    rows = []
    for (name, (_, expression)), outcome in zip(selected, expected):
        assert outcome.get('error') in (None, 'TypeError'), (name, outcome)
        data = 'None' if 'error' in outcome else f'Some(&{outcome["bytes"]})'
        rows.append(f'({json.dumps(name)}, {expression}, {data})')
    module = json.dumps(str(ROOT/'crates/framer-native/src/castle/mod.rs'))
    source = f'''#[allow(dead_code,unused_imports)] #[path={module}] mod castle;
use castle::v3::{{Value, CollectorResult, CollectorError, NetworkValueError,
    apply_collector_result, serialize}};
fn base() -> Vec<Value> {{ (1000..1765).map(|n|Value::Number(n as f64)).collect() }}
#[test] fn original_hash_assignment_and_preserved_positions() {{
    let cases:[(&str,Value,Option<&[u8]>);{len(rows)}]=[{','.join(rows)}];
    for (name,input,expected) in cases {{
        let mut values=base(); let before=serialize(&values).unwrap();
        let result=apply_collector_result(&mut values,CollectorResult::NetworkHash(input));
        match expected {{
            Some(bytes)=>{{
                result.unwrap();
                let value=std::mem::replace(&mut values[469],Value::Number(1469.0));
                assert_eq!(serialize(&[value]).unwrap(),bytes,"{{name}}");
            }},
            None=>assert_eq!(result,Err(CollectorError::Network(NetworkValueError::MissingRecord))),
        }}
        assert_eq!(serialize(&values).unwrap(),before,"{{name}}");
    }}
}}
'''
    source += refusal_tests()
    with tempfile.TemporaryDirectory(prefix='v3-network-hash-') as directory:
        path = Path(directory)
        (path/'check.rs').write_text(source)
        compile_fixture = runpy.run_path(str(ROOT/'scripts/v3_native_fixture.py'))['compile_fixture']
        compile_fixture(path/'check.rs', path/'check')
        subprocess.run([str(path/'check'), '--test-threads=1'], check=True, timeout=10)
    errors = sum('error' in row for row in expected)
    print(f'PASS: {len(selected)} original hash assignments, including {errors} missing-row errors')


def refusal_tests():
    return '''
fn record(value:Value) -> Value {
    let mut row:Vec<_>=(0..47).map(|_|Value::Undefined).collect();
    row.push(value); Value::Array(vec![Value::Array(row)])
}
#[test] fn refuses_erased_types_and_non_string_hashes_without_mutation() {
    for input in [Value::JsonFailure, Value::JsonFallback(vec![48]),
        Value::Array(vec![Value::JsonFailure]), record(Value::JsonFailure),
        record(Value::JsonFallback(vec![48])), record(Value::Bool(true)),
        record(Value::Number(1.0)), record(Value::Array(vec![])),
        record(Value::Object(vec![]))] {
        let mut values=base(); let before=serialize(&values).unwrap();
        assert!(apply_collector_result(&mut values,CollectorResult::NetworkHash(input)).is_err());
        assert_eq!(serialize(&values).unwrap(),before);
    }
}
#[test] fn missing_target_position_preserves_the_array() {
    let mut values=base(); values.truncate(469); let before=serialize(&values).unwrap();
    assert_eq!(apply_collector_result(&mut values,CollectorResult::NetworkHash(Value::Null)),
        Err(CollectorError::MissingPosition(469)));
    assert_eq!(serialize(&values).unwrap(),before);
}
'''


if __name__ == '__main__':
    check()
